use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

const PREVIEW_MATCHES: usize = 5;
const CONTEXT_CHARS: usize = 38;

pub(crate) fn find_next(body: &str, query: &str, after_utf16: usize) -> Option<(usize, usize)> {
    if query.is_empty() {
        return None;
    }
    let mut units = 0;
    let mut cursor = body.len();
    for (byte, ch) in body.char_indices() {
        if units >= after_utf16 {
            cursor = byte;
            break;
        }
        units += ch.len_utf16();
    }
    let index = body[cursor..].find(query).map(|offset| cursor + offset).or_else(|| body.find(query))?;
    let start = body[..index].encode_utf16().count();
    Some((start, start + query.encode_utf16().count()))
}

pub(crate) struct Replacement {
    pub body: String,
    pub matches: usize,
    pub preview: String,
}

pub(crate) fn plan_replacement(
    body: &str, query: &str, replacement: &str, max_bytes: usize,
) -> Result<Replacement, &'static str> {
    if query.is_empty() || query.contains('\0') || replacement.contains('\0') {
        return Err("請輸入要尋找的文字。");
    }
    let replacement = adapt_newlines(body, replacement);
    let matches = body.matches(query).count();
    let size = replacement.len().checked_mul(matches)
        .and_then(|bytes| bytes.checked_add(body.len() - query.len() * matches))
        .filter(|size| *size <= max_bytes).ok_or("全部取代後會超過筆記大小上限。")?;
    let mut result = String::with_capacity(size);
    let mut preview = format!("共有 {matches} 處完全相符，區分大小寫。");
    let mut previous = 0;
    for (index, (start, _)) in body.match_indices(query).enumerate() {
        let end = start + query.len();
        result.push_str(&body[previous..start]);
        result.push_str(&replacement);
        previous = end;
        if index < PREVIEW_MATCHES {
            let left = body[..start].char_indices().rev().nth(CONTEXT_CHARS - 1)
                .map_or(0, |(byte, _)| byte);
            let right = end + body[end..].char_indices().nth(CONTEXT_CHARS)
                .map_or(body.len() - end, |(byte, _)| byte);
            let before = format!("{}{}{}", &body[left..start], query, &body[end..right]);
            let after = format!("{}{}{}", &body[left..start], replacement, &body[end..right]);
            use std::fmt::Write;
            let _ = write!(preview, "\n\n第 {} 處\n原文　{}\n取代　{}", index + 1,
                display_excerpt(&before), display_excerpt(&after));
        }
    }
    result.push_str(&body[previous..]);
    if matches > PREVIEW_MATCHES {
        use std::fmt::Write;
        let _ = write!(preview, "\n\n另有 {} 處也會依同一條件取代。", matches - PREVIEW_MATCHES);
    }
    Ok(Replacement { body: result, matches, preview })
}

fn display_excerpt(text: &str) -> String {
    let line = text.replace("\r\n", "\n").replace(['\r', '\n'], "↵");
    if let Some((end, _)) = line.char_indices().nth(160) {
        format!("{}…", &line[..end])
    } else {
        line
    }
}

fn adapt_newlines(body: &str, replacement: &str) -> String {
    let normal = replacement.replace("\r\n", "\n").replace('\r', "\n");
    if !normal.contains('\n') {
        return normal;
    }
    let mut pair = 0;
    let mut cr = 0;
    let mut lf = 0;
    let mut bytes = body.bytes().peekable();
    while let Some(byte) = bytes.next() {
        match byte {
            b'\r' if bytes.peek() == Some(&b'\n') => { bytes.next(); pair += 1; }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
    }
    match (pair, cr, lf) {
        (1.., 0, 0) => normal.replace('\n', "\r\n"),
        (0, 1.., 0) => normal.replace('\n', "\r"),
        _ => normal,
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct WordCount {
    pub words: usize,
    pub characters: usize,
}

pub(crate) fn count_prose(body: &str) -> WordCount {
    let mut total = WordCount::default();
    let mut inside_code = false;
    let mut in_word = false;
    for event in Parser::new_ext(body, Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_WIKILINKS | Options::ENABLE_GFM)
    {
        match event {
            Event::Start(Tag::CodeBlock(_)) => { inside_code = true; in_word = false; }
            Event::End(TagEnd::CodeBlock) => { inside_code = false; in_word = false; }
            Event::Text(text) if !inside_code => {
                for ch in text.chars() {
                    total.characters += usize::from(!ch.is_whitespace());
                    if is_individual_word(ch) {
                        total.words += 1;
                        in_word = false;
                    } else if ch.is_alphanumeric() {
                        total.words += usize::from(!in_word);
                        in_word = true;
                    } else if !matches!(ch, '\u{0300}'..='\u{036f}') {
                        in_word = false;
                    }
                }
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item)
            | Event::Code(_) | Event::Html(_) | Event::SoftBreak | Event::HardBreak
            | Event::Rule | Event::TaskListMarker(_) => in_word = false,
            _ => {}
        }
    }
    total
}

fn is_individual_word(ch: char) -> bool {
    matches!(ch, '\u{3041}'..='\u{3096}' | '\u{30a1}'..='\u{30fa}'
        | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{ac00}'..='\u{d7a3}'
        | '\u{f900}'..='\u{faff}' | '\u{20000}'..='\u{2fa1f}' | '\u{30000}'..='\u{323af}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_exact_text_from_utf16_caret_and_wraps() {
        let body = "😀𠮷\r\n字 Foo 字 Foo foo";
        assert_eq!(find_next(body, "字", 0), Some((6, 7)));
        assert_eq!(find_next(body, "字", 7), Some((12, 13)));
        assert_eq!(find_next(body, "字", 13), Some((6, 7)));
        assert_eq!(find_next(body, "𠮷", 3), Some((2, 4)));
        assert_eq!(find_next(body, "FOO", 0), None);
        assert_eq!(find_next(body, "", 0), None);
    }

    #[test]
    fn plans_bounded_nonoverlapping_literal_replacements_without_rewriting_other_lines() {
        let source = "😀.字\r\n字 .字\r\n.字字 .字 .字 .字";
        let plan = plan_replacement(source, ".字", "新\n值", 4096).unwrap();
        assert_eq!(plan.matches, 6);
        assert_eq!(plan.body, "😀新\r\n值\r\n字 新\r\n值\r\n新\r\n值字 新\r\n值 新\r\n值 新\r\n值");
        assert!(plan.preview.contains("另有 1 處"));
        assert_eq!(plan_replacement("aaaaa", "aa", "", 10).unwrap().body, "a");
        assert_eq!(plan_replacement("same", "absent", "effect", 4).unwrap().matches, 0);
        assert!(plan_replacement("aaa", "a", "字", 8).is_err());
        assert!(plan_replacement("unchanged", "","",20).is_err());
    }

    #[test]
    fn replacement_preview_cannot_overwrite_a_newer_saved_note() -> crate::store::Result<()> {
        let path = std::env::temp_dir().join(format!("paperroom-replace-{}.sqlite3", std::process::id()));
        {
            let mut first = crate::store::Notebook::open(&path, false)?;
            let other = crate::store::Notebook::open(&path, false)?;
            let id = first.add(None, "版本測試", "😀舊值\r\n舊值")?;
            let note = first.note(id)?.ok_or("note missing")?;
            let plan = plan_replacement(&note.body, "舊值", "新值", crate::store::MAX_NOTE_BYTES)?;
            assert_eq!(plan.matches, 2);
            other.save(id, note.version, &note.row.title, "另一個視窗的內容")?;
            assert!(first.save(id, note.version, &note.row.title, &plan.body).is_err());
            assert_eq!(first.note(id)?.unwrap().body, "另一個視窗的內容");
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn counts_visible_markdown_prose_and_ignores_code_markers_urls_and_html_tags() {
        let note = "# Hello 世界\n\n**mixed** [label](https://example.test/ignore) `ignore`\n\n\
            > 日本語 123\n\n```text\nhidden fenced prose\n```\n\n\
            <!-- hidden HTML block -->\n\n<em>shown</em> cafe\u{301}";
        let stats = count_prose(note);
        assert_eq!(stats.words, 11);
        assert_eq!(stats.characters, 33);
    }
}
