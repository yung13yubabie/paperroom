use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use std::fmt::Write;

pub fn preview(body: &str) -> String {
    let mut out = String::from(concat!(
        r"{\rtf1\ansi\uc1\deff0",
        r"{\fonttbl{\f0 Microsoft JhengHei UI;}{\f1 Consolas;}}",
        r"{\colortbl;\red24\green43\blue38;\red27\green103\blue86;",
        r"\red94\green109\blue101;\red132\green90\blue56;}",
        r"\viewkind4\pard\sl342\slmult1\sa170\f0\fs25\cf1 ",
    ));
    let parser = Parser::new_ext(
        body,
        Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_WIKILINKS,
    );
    let mut list = Vec::<Option<u64>>::new();
    for event in parser {
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
            Event::Start(Tag::BlockQuote(_)) => out.push_str(r"{\li330\i\cf3 "),
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
            Event::Text(text) | Event::InlineHtml(text) | Event::Html(text) => {
                rtf_text(&mut out, &text)
            }
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
