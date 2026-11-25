#[path = "note_text_tools.rs"]
pub(crate) mod note_text_tools;
#[path = "block_anchors.rs"]
pub(crate) mod block_anchors;

use pulldown_cmark::{BlockQuoteKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use std::fmt::Write;
use std::ops::Range;

pub fn preview(body: &str) -> String {
    preview_marked(body, &[])
}

pub(crate) fn preview_marked(body: &str, marks: &[Range<usize>]) -> String {
    let mut out = String::from(concat!(
        r"{\rtf1\ansi\uc1\deff0",
        r"{\fonttbl{\f0 Microsoft JhengHei UI;}{\f1 Consolas;}}",
        r"{\colortbl;\red24\green43\blue38;\red27\green103\blue86;",
        r"\red94\green109\blue101;\red132\green90\blue56;",
    ));
    if !marks.is_empty() {
        out.push_str(r"\red255\green232\blue131;");
    }
    out.push_str(r"}\viewkind4\pard\sl342\slmult1\sa170\f0\fs25\cf1 ");
    let parser = Parser::new_ext(
        body,
        Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_WIKILINKS
            | Options::ENABLE_GFM,
    );
    let mut list = Vec::<Option<u64>>::new();
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let size = match level {
                    HeadingLevel::H1 => 41,
                    HeadingLevel::H2 => 34,
                    HeadingLevel::H3 => 30,
                    _ => 27,
                };
                let _ = write!(out, "{{\\sb190\\sa135\\b\\cf2\\fs{size} ");
            }
            Event::End(TagEnd::Heading(_)) => out.push_str(r"\par}"),
            Event::Start(Tag::List(start)) => list.push(start),
            Event::End(TagEnd::List(_)) => {
                list.pop();
                out.push_str(r"\par ");
            }
            Event::Start(Tag::Item) => {
                let _ = write!(out, "{{\\li{}\\sa70 ", list.len() * 275);
                if let Some(Some(next)) = list.last_mut() {
                    let _ = write!(out, "{next}.  ");
                    *next += 1;
                } else {
                    out.push_str(r"\bullet  ");
                }
            }
            Event::End(TagEnd::Item) => out.push_str(r"\par}"),
            Event::Start(Tag::BlockQuote(Some(kind))) => {
                let (title, color) = match kind {
                    BlockQuoteKind::Note => ("筆記", 2),
                    BlockQuoteKind::Tip => ("提示", 2),
                    BlockQuoteKind::Important => ("重要", 4),
                    BlockQuoteKind::Warning => ("警告", 4),
                    BlockQuoteKind::Caution => ("注意", 4),
                };
                let _ = write!(out, r"{{\li330\brdrl\brdrs\brdrw22\brdrcf{color}\brsp80 {{\b\cf{color} ");
                rtf_text(&mut out, title);
                out.push_str(r"}\line ");
            }
            Event::Start(Tag::BlockQuote(None)) => out.push_str(r"{\li330\i\cf3 "),
            Event::End(TagEnd::BlockQuote(_)) => out.push('}'),
            Event::Start(Tag::CodeBlock(_)) => out.push_str(r"{\f1\cf3\li235 "),
            Event::End(TagEnd::CodeBlock) => out.push_str(r"\par}"),
            Event::Start(Tag::Strong) => out.push_str(r"{\b "),
            Event::Start(Tag::Emphasis) => out.push_str(r"{\i "),
            Event::Start(Tag::Strikethrough) => out.push_str(r"{\strike "),
            Event::Start(Tag::Link { .. }) => out.push_str(r"{\ul\cf2 "),
            Event::Start(Tag::Image { .. }) => out.push_str(r"{\i\cf3 "),
            Event::End(
                TagEnd::Strong
                | TagEnd::Emphasis
                | TagEnd::Strikethrough
                | TagEnd::Link
                | TagEnd::Image,
            ) => out.push('}'),
            Event::End(TagEnd::Paragraph) if list.is_empty() => out.push_str(r"\par "),
            Event::Text(text) => {
                if let Some(parts) = highlight_parts(body, &text, range, marks) {
                    for (highlighted, part) in parts {
                        if highlighted { out.push_str(r"{\highlight5 "); }
                        rtf_text(&mut out, part);
                        if highlighted { out.push('}'); }
                    }
                } else {
                    rtf_text(&mut out, &text);
                }
            }
            Event::InlineHtml(text) | Event::Html(text) => rtf_text(&mut out, &text),
            Event::Code(text) => {
                out.push_str(r"{\f1\cf4 ");
                rtf_text(&mut out, &text);
                out.push('}');
            }
            Event::SoftBreak | Event::HardBreak => out.push_str(r"\line "),
            Event::Rule => out.push_str(r"{\cf3 \u8212?\u8212?\u8212?\par}"),
            Event::TaskListMarker(done) => {
                out.push_str(if done { r"\u9745?  " } else { r"\u9744?  " })
            }
            _ => {}
        }
    }
    out.push('}');
    out
}

pub(crate) fn highlight_parts<'a>(
    body: &str, text: &'a str, raw: Range<usize>, marks: &[Range<usize>],
) -> Option<Vec<(bool, &'a str)>> {
    let first = marks.partition_point(|mark| mark.end <= raw.start);
    if marks.get(first)?.start >= raw.end || body.get(raw.clone()) != Some(text) {
        return None;
    }
    let mut parts = Vec::new();
    let mut offset = 0;
    for mark in marks[first..].iter().take_while(|mark| mark.start < raw.end) {
        if mark.start < raw.start || mark.end > raw.end || mark.end <= mark.start {
            continue;
        }
        let start = mark.start - raw.start;
        let end = mark.end - raw.start;
        if let (Some(prefix), Some(selected)) = (text.get(offset..start), text.get(start..end)) {
            parts.push((false, prefix));
            parts.push((true, selected));
            offset = end;
        }
    }
    if parts.is_empty() { return None; }
    parts.push((false, &text[offset..]));
    Some(parts)
}

pub fn wiki_targets(body: &str) -> Vec<String> {
    Parser::new_ext(body, Options::ENABLE_WIKILINKS)
        .filter_map(|event| {
            if let Event::Start(Tag::Link {
                link_type: LinkType::WikiLink { .. },
                dest_url,
                ..
            }) = event
            {
                let name = dest_url.trim();
                return (!name.is_empty()).then(|| name.to_owned());
            }
            None
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AnchorSyntax {
    Wiki,
    Markdown,
}

pub(crate) struct NoteReference {
    pub destination: String,
    pub label: String,
    pub syntax: AnchorSyntax,
    pub offset: usize,
}

/// Return parsed WikiLinks and Markdown links, excluding literal code.
pub(crate) fn note_references(body: &str) -> Vec<NoteReference> {
    let mut active = None::<NoteReference>;
    let mut references = Vec::new();
    for (event, range) in Parser::new_ext(body, Options::ENABLE_WIKILINKS).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { link_type, dest_url, .. }) => {
                let syntax = if matches!(link_type, LinkType::WikiLink { .. }) {
                    AnchorSyntax::Wiki
                } else {
                    AnchorSyntax::Markdown
                };
                active = Some(NoteReference {
                    destination: dest_url.trim().to_owned(), label: String::new(), syntax, offset: range.start,
                });
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(reference) = &mut active {
                    reference.label.push_str(&text);
                }
            }
            Event::End(TagEnd::Link) => {
                if let Some(reference) = active.take().filter(|item| !item.destination.is_empty()) {
                    references.push(reference);
                }
            }
            _ => {}
        }
    }
    references
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum HeadingLookup {
    Found { ordinal: usize, section: std::ops::Range<usize> },
    Missing,
    Ambiguous,
    Unsupported,
}

fn decode_fragment(raw: &str) -> Option<String> {
    let mut source = raw.bytes();
    let mut decoded = Vec::with_capacity(raw.len());
    while let Some(byte) = source.next() {
        let byte = if byte == b'%' {
            let high = char::from(source.next()?).to_digit(16)?;
            let low = char::from(source.next()?).to_digit(16)?;
            (high * 16 + low) as u8
        } else {
            byte
        };
        decoded.push(byte);
    }
    String::from_utf8(decoded).ok()
}

fn heading_levels(anchor: &str, syntax: AnchorSyntax) -> Option<Vec<String>> {
    let suffix = anchor.strip_prefix('#')?;
    if suffix.len() > 4096 { return None; }
    // Split before decoding: "%23" is a literal hash in one Markdown
    // heading. A raw "#" separates explicitly named ancestors.
    suffix.split('#').map(|part| {
        if part.starts_with('^') { return None; }
        let name = match syntax {
            AnchorSyntax::Wiki => part.to_owned(),
            AnchorSyntax::Markdown => decode_fragment(part)?,
        };
        let label = name.trim();
        (!label.is_empty() && !name.chars().any(char::is_control)).then(|| label.to_owned())
    }).collect()
}

fn slug_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_')
}

fn heading_slug(title: &str) -> String {
    title.trim().to_lowercase().chars().filter_map(|c| {
        if c.is_whitespace() { Some('-') }
        else if slug_char(c) { Some(c) }
        else { None }
    }).collect()
}

fn find_heading(headings: &[(usize, u8, String)], levels: &[String], slugs: bool, bytes: usize) -> HeadingLookup {
    let Some((last, parents)) = levels.split_last() else { return HeadingLookup::Unsupported; };
    let same = |title: &str, query: &str| {
        title.trim() == query || (slugs && query.chars().all(slug_char)
            && heading_slug(title) == query.to_lowercase())
    };
    let mut ancestry = Vec::<usize>::new();
    let mut found = None;
    for (ordinal, (start, level, title)) in headings.iter().enumerate() {
        while ancestry.last().is_some_and(|&i| headings[i].1 >= *level) {
            ancestry.pop();
        }
        if same(title, last) {
            let mut chain = ancestry.iter().rev();
            if parents.iter().rev().all(|parent| chain.any(|&i| same(&headings[i].2, parent))) {
                if found.is_some() { return HeadingLookup::Ambiguous; }
                found = Some((ordinal, *start, *level));
            }
        }
        ancestry.push(ordinal);
    }
    let Some((ordinal, start, level)) = found else { return HeadingLookup::Missing; };
    let end = headings[ordinal + 1..].iter().find(|(_, depth, _)| *depth <= level)
        .map_or(bytes, |(position, _, _)| *position);
    HeadingLookup::Found { ordinal, section: start..end }
}

pub(crate) fn resolve_heading(body: &str, anchor: &str, syntax: AnchorSyntax) -> HeadingLookup {
    let Some(levels) = heading_levels(anchor, syntax) else { return HeadingLookup::Unsupported; };
    let headings = source_headings(body);
    let exact = find_heading(&headings, &levels, false, body.len());
    if exact == HeadingLookup::Missing && syntax == AnchorSyntax::Markdown {
        find_heading(&headings, &levels, true, body.len())
    } else {
        exact
    }
}

/// Resolve a unique literal heading or explicit chain, preserving source bytes.
pub fn heading_section<'a>(body: &'a str, anchor: &str) -> Option<&'a str> {
    match resolve_heading(body, anchor, AnchorSyntax::Wiki) {
        HeadingLookup::Found { section, .. } => body.get(section),
        _ => None,
    }
}

pub(crate) fn source_headings(body: &str) -> Vec<(usize, u8, String)> {
    let (_, prose) = crate::store::vault_search::frontmatter(body);
    let skipped = body.len() - prose.len();
    heading_offsets(prose, Options::empty()).into_iter()
        .map(|(at, level, title)| (skipped + at, level, title)).collect()
}

pub(crate) fn heading_offsets(body: &str, options: Options) -> Vec<(usize, u8, String)> {
    let mut headings = Vec::new();
    let mut active = None::<(usize, u8, String)>;
    for (event, range) in Parser::new_ext(body, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                active = Some((range.start, level as u8, String::new()));
            }
            Event::Text(value) | Event::Code(value) => {
                if let Some((_, _, text)) = &mut active { text.push_str(&value); }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(heading) = active.take() { headings.push(heading); }
            }
            _ => {}
        }
    }
    headings
}

fn rtf_text(out: &mut String, text: &str) {
    for unit in text.encode_utf16() {
        match unit {
            10 => out.push_str(r"\line "),
            13 => {}
            9 => out.push_str(r"\tab "),
            92 | 123 | 125 => {
                out.push('\\');
                out.push(char::from(unit as u8));
            }
            32..=126 => out.push(char::from(unit as u8)),
            _ => {
                let _ = write!(out, "\\u{}?", unit as i16);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn anchors_distinguish_nested_headings_from_one_percent_encoded_hash() {
        let text = concat!(
            "---\r\naliases: [來源]\r\n---\r\n", "📷 前文\r\n",
            "# 第一幕\r\n## 同名\r\n錯誤分支\r\n", "# 第二幕\r\n### 過場\r\n#### 同名\r\n正確分支\r\n",
            "# 第二幕#同名\r\n含雜湊符號\r\n", "# 鏡位%23A\r\n字面百分號\r\n",
            "# North Take!\r\n標準網址片段\r\n",
        );
        let locate = |anchor, syntax| match resolve_heading(text, anchor, syntax) {
            HeadingLookup::Found { section, .. } => &text[section],
            _ => panic!("missing {anchor}"),
        };
        assert_eq!(resolve_heading(text, "#同名", AnchorSyntax::Wiki), HeadingLookup::Ambiguous);
        assert!(locate("#第二幕#同名", AnchorSyntax::Wiki).contains("正確分支"));
        assert!(locate("#第二幕%23同名", AnchorSyntax::Markdown).contains("含雜湊符號"));
        assert!(locate("#鏡位%23A", AnchorSyntax::Wiki).contains("字面百分號"));
        assert!(locate("#鏡位%2523A", AnchorSyntax::Markdown).contains("字面百分號"));
        assert!(locate("#north-take", AnchorSyntax::Markdown).contains("標準網址片段"));
        assert_eq!(resolve_heading(text, "#不存在#同名", AnchorSyntax::Wiki), HeadingLookup::Missing);
        assert_eq!(resolve_heading(text, "#第一幕##同名", AnchorSyntax::Wiki), HeadingLookup::Unsupported);
        assert_eq!(resolve_heading(text, "#test%2", AnchorSyntax::Markdown), HeadingLookup::Unsupported);
        assert_eq!(resolve_heading(text, "#^無區塊", AnchorSyntax::Wiki), HeadingLookup::Unsupported);
        assert_eq!(resolve_heading(text, "#north-take", AnchorSyntax::Wiki), HeadingLookup::Missing);
    }

    #[test]
    fn rtf_keeps_unicode_and_escapes_literal_control_text() {
        let text =
            preview("## 中文\n\n- [x] 已做\n\n`{\\rtf}`\n\n![圖](https://example.invalid/no)");
        assert!(text.contains(r"\fs34"));
        assert!(text.contains(r"\u20013?"));
        assert!(text.contains(r"\u9745?"));
        assert!(text.contains(r"\{\\rtf\}"));
        assert!(!text.contains("https://"));
    }
}

#[cfg(test)]
#[test]
fn displays_standard_callouts_and_keeps_unknown_markers_literal() {
    let body = "> [!warning]\n> Stand clear **now**.\n\n> [!custom] My label\n> original";
    let rendered = preview(body);
    assert!(rendered.contains(r"\brdrcf4"));
    assert!(rendered.contains("Stand clear"));
    assert!(rendered.contains("{\\b now}"));
    assert!(rendered.contains("[!custom] My label"));
    assert!(!rendered.contains("[!warning]"));
}

#[cfg(test)]
mod note_format_roundtrip_tests {
    #[test]
    fn note_format_survives_storage_reading_view_and_markdown_export() -> crate::store::Result<()> {
        use crate::store::Notebook;
        use std::fs;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = loop {
            let candidate = std::env::temp_dir().join(format!(
                "paperroom-inline-format-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        };
        let result = (|| -> crate::store::Result<()> {
            let workspace = dir.join("notes.sqlite3");
            let export = dir.join("sample.md");
            let title = "筆記格式保留";
            let body = concat!(
                "**Bold 粗😀** and *Italic 斜* then ~~Strike 刪~~.\n\n",
                "==literal-mark== and `==code-mark==` and \\=\\=escaped-mark\\=\\=.\n",
            );
            let mut notebook = Notebook::open(&workspace, false)?;
            let id = notebook.add(None, title, "initial")?;
            let version = notebook.note(id)?.ok_or("initial note missing")?.version;
            let saved = notebook.save(id, version, title, body)?;
            drop(notebook);

            let notebook = Notebook::open(&workspace, false)?;
            let note = notebook.note(id)?.ok_or("saved note missing")?;
            assert_eq!(note.version, saved);
            assert_eq!(note.row.title, title);
            assert_eq!(note.body.as_bytes(), body.as_bytes());
            let rtf = super::preview(&note.body);
            assert!(rtf.contains(r"{\b Bold ") && rtf.contains(r"{\i Italic ")
                 && rtf.contains(r"{\strike Strike "));
            assert!(rtf.contains("==literal-mark==") && rtf.contains("==escaped-mark=="));
            assert!(rtf.contains(r"{\f1\cf4 ==code-mark==}"));
            crate::archive::export_note(&export, &note.row.title, &note.body)?;
            assert_eq!(fs::read(&export)?, format!("# {title}\n\n{body}").as_bytes());
            let imported = crate::archive::markdown(&export)?;
            assert_eq!(imported.title, title);
            assert_eq!(imported.body.as_bytes(), body.as_bytes());
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }
}
