//! Explicit Obsidian block references use source offsets, never guessed text matches.
use pulldown_cmark::{Event, Options, Parser, TagEnd};
use std::{collections::HashMap, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockKind {
    Paragraph,
    ListItem,
    List,
    Quote,
    Callout,
    Table,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BlockLookup {
    Found { source: Range<usize>, kind: BlockKind },
    Missing,
    Ambiguous,
    Unsupported,
}

struct Frame {
    kind: TagEnd,
    source: Range<usize>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 4096
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn line_breaks(text: &str) -> usize {
    text.bytes().filter(|&byte| byte == b'\n').count()
        + text.bytes().filter(|&byte| byte == b'\r').count()
        - text.match_indices("\r\n").count()
}

fn list_item_line(stack: &[Frame]) -> bool {
    matches!(stack.last().map(|frame| frame.kind), Some(TagEnd::Item | TagEnd::Paragraph))
        && stack.iter().any(|frame| frame.kind == TagEnd::Item)
        && stack.iter().all(|frame| matches!(frame.kind, TagEnd::List(_) | TagEnd::Item | TagEnd::Paragraph))
}

/// A plain text offset is needed so a token in a code span, formatted label,
/// link destination, or escaped syntax is never mistaken for a block ID.
fn inline_markers<'a>(
    prose: &'a str, range: Range<usize>, stack: &[Frame],
    mut visit: impl FnMut(&'a str, Range<usize>, BlockKind) -> bool,
) -> bool {
    let Some(frame) = stack.last() else { return false; };
    let paragraph = stack.len() == 1 && frame.kind == TagEnd::Paragraph;
    if !paragraph && !list_item_line(stack) {
        return false;
    }
    let text = &prose[range.clone()];
    for (offset, _) in text.match_indices('^') {
        let start = range.start + offset;
        let tail = &text[offset + 1..];
        let end_id = tail.find(|c: char| !c.is_ascii_alphanumeric() && c != '-').unwrap_or(tail.len());
        let id = &tail[..end_id];
        if !valid_id(id) {
            continue;
        }
        let end = start + 1 + end_id;
        let before = &prose[..start];
        if !before.ends_with([' ', '\t']) {
            continue;
        }
        let trailing_space_only = if paragraph {
            prose[end..frame.source.end].chars().all(char::is_whitespace)
        } else {
            prose[end..].chars().take_while(|c| !matches!(*c, '\r' | '\n')).all(char::is_whitespace)
        };
        if !trailing_space_only {
            continue;
        }
        let line_start = before.rfind(['\r', '\n']).map_or(0, |newline| newline + 1);
        if prose[line_start..start].trim().is_empty() {
            continue;
        }
        let begin = if paragraph { frame.source.start } else { line_start };
        let content = prose[begin..start].trim_end_matches([' ', '\t']);
        let kind = if paragraph { BlockKind::Paragraph } else { BlockKind::ListItem };
        if visit(id, begin..begin + content.len(), kind) {
            return true;
        }
    }
    false
}

fn standalone_marker(
    prose: &str, marker_source: &Range<usize>, previous: Option<(Range<usize>, BlockKind)>,
) -> Option<(Range<usize>, BlockKind)> {
    let (previous, kind) = previous?;
    let end = previous.start + prose[previous.clone()].trim_end().len();
    let gap = &prose[end..marker_source.start];
    let rest = &prose[marker_source.start + prose[marker_source.clone()].trim_end().len()..];
    // Structured blocks and their standalone ID are separated by a blank
    // line. End of file can take the place of the following blank line.
    let after = &rest[..rest.len() - rest.trim_start().len()];
    if gap.trim().is_empty() && line_breaks(gap) >= 2
        && (rest.trim().is_empty() || line_breaks(after) >= 2)
    {
        Some((previous.start..end, kind))
    } else {
        None
    }
}

fn visit_block_markers<'a>(body: &'a str, mut visit: impl FnMut(&'a str, Range<usize>, BlockKind) -> bool) {
    let (_, prose) = crate::store::vault_search::frontmatter(body);
    let base = body.len() - prose.len();
    let mut include = |id, source: Range<usize>, kind| visit(id, base + source.start..base + source.end, kind);
    let mut stack = Vec::<Frame>::new();
    let mut previous = None;
    let opts = Options::ENABLE_WIKILINKS | Options::ENABLE_TABLES | Options::ENABLE_GFM;
    for (event, range) in Parser::new_ext(prose, opts).into_offset_iter() {
        match event {
            Event::Start(tag) => stack.push(Frame { kind: tag.to_end(), source: range }),
            Event::End(tag) => {
                stack.pop();
                if !stack.is_empty() {
                    continue;
                }
                match tag {
                    TagEnd::List(_) => previous = Some((range, BlockKind::List)),
                    TagEnd::BlockQuote(kind) => {
                        let kind = if kind.is_some() { BlockKind::Callout } else { BlockKind::Quote };
                        previous = Some((range, kind));
                    }
                    TagEnd::Table => previous = Some((range, BlockKind::Table)),
                    TagEnd::Paragraph => {
                        let prior = previous.take();
                        if let Some(id) = prose[range.clone()].trim().strip_prefix('^').filter(|id| valid_id(id))
                            && let Some((source, kind)) = standalone_marker(prose, &range, prior)
                            && include(id, source, kind)
                        {
                            return;
                        }
                    }
                    _ => previous = None,
                }
            }
            Event::Text(_) => {
                if inline_markers(prose, range, &stack, &mut include) { return; }
            }
            _ => {}
        }
    }
}

impl BlockLookup {
    fn record(&mut self, source: Range<usize>, kind: BlockKind) {
        *self = if *self == Self::Missing { Self::Found { source, kind } } else { Self::Ambiguous };
    }
}

pub(crate) fn resolve_block(body: &str, anchor: &str) -> BlockLookup {
    let Some(id) = anchor.strip_prefix("#^").filter(|id| valid_id(id)) else {
        return BlockLookup::Unsupported;
    };
    let mut found = BlockLookup::Missing;
    visit_block_markers(body, |candidate, source, kind| {
        if candidate == id { found.record(source, kind); }
        found == BlockLookup::Ambiguous
    });
    found
}

/// Resolve just the requested IDs using the same syntax and one parse of the saved Markdown.
pub(crate) fn resolve_blocks<'a>(body: &str, ids: impl IntoIterator<Item = &'a str>) -> HashMap<&'a str, BlockLookup> {
    let mut found = ids.into_iter().map(|id| {
        (id, if valid_id(id) { BlockLookup::Missing } else { BlockLookup::Unsupported })
    }).collect::<HashMap<_, _>>();
    if found.values().any(|state| *state == BlockLookup::Missing) {
        visit_block_markers(body, |id, source, kind| {
            if let Some(result) = found.get_mut(id) {
                result.record(source, kind);
            }
            false
        });
    }
    found
}


pub(crate) fn has_block_markers(body: &str) -> bool {
    let mut found = false;
    visit_block_markers(body, |_, _, _| {
        found = true;
        true
    });
    found
}

/// Find explicitly authored IDs and keep only those that resolve to one block.
pub(crate) fn unique_block_sources(body: &str) -> Vec<(&str, Range<usize>)> {
    let mut found = HashMap::new();
    visit_block_markers(body, |id, source, kind| {
        found.entry(id).or_insert(BlockLookup::Missing).record(source, kind);
        false
    });
    let mut sources = found.into_iter().filter_map(|(id, lookup)| {
        match lookup {
            BlockLookup::Found { source, .. } => Some((id, source)),
            _ => None,
        }
    }).collect::<Vec<_>>();
    sources.sort_unstable_by_key(|(id, range)| (range.start, *id));
    sources
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_paragraph_list_item_and_standalone_structured_blocks() {
        let body = concat!(
            "\u{feff}---\r\naliases: [隱藏]\r\n木 ^meta\r\n---\r\n\r\n",
            "🎬 行一\r\n行二 **加粗** [文字](#heading) `鏡位` ^part-A\r\n\r\n",
            "- 第一項 ^single\r\n- 第二項\r\n  - 巢狀項目 ^nest\r\n\r\n^all-items\r\n\r\n",
            "> 引文留存\r\n> 第二行 ^inside-quote\r\n\r\n^quoted\r\n\r\n",
            "> [!NOTE]\r\n> 保留提醒\r\n\r\n^notice\r\n\r\n",
            "|頭|尾|\r\n|---|---|\r\n|列|資料|\r\n\r\n^tabular",
        );
        let found = |anchor| match resolve_block(body, anchor) {
            BlockLookup::Found { source, kind } => (&body[source], kind),
            other => panic!("{anchor}: {other:?}"),
        };
        assert_eq!(found("#^part-A"),
            ("🎬 行一\r\n行二 **加粗** [文字](#heading) `鏡位`", BlockKind::Paragraph));
        assert_eq!(found("#^single"), ("- 第一項", BlockKind::ListItem));
        assert_eq!(found("#^nest"), ("  - 巢狀項目", BlockKind::ListItem));
        assert_eq!(found("#^all-items"),
            ("- 第一項 ^single\r\n- 第二項\r\n  - 巢狀項目 ^nest", BlockKind::List));
        assert_eq!(found("#^quoted"), ("> 引文留存\r\n> 第二行 ^inside-quote", BlockKind::Quote));
        assert_eq!(found("#^notice"), ("> [!NOTE]\r\n> 保留提醒", BlockKind::Callout));
        assert_eq!(found("#^tabular"), ("|頭|尾|\r\n|---|---|\r\n|列|資料|", BlockKind::Table));
        for anchor in ["#^meta", "#^inside-quote", "#^part-a", "#^part"] {
            assert_eq!(resolve_block(body, anchor), BlockLookup::Missing, "{anchor}");
        }
    }

    #[test]
    fn bulk_resolution_has_the_same_exact_status_as_interactive_preview() {
        let body = "正文 ^one\n\n- 整個 **短清單**\n\n^list\n\n撞名 ^dup\n\n- 又一筆 ^dup";
        let ids = ["one", "list", "dup", "Dup", "missing", "bad_id", "one"];
        let batch = resolve_blocks(body, ids);
        assert_eq!(batch.len(), 6);
        for id in ids {
            assert_eq!(batch[id], resolve_block(body, &format!("#^{id}")), "{id}");
        }
    }

    #[test]
    fn rejects_markup_escapes_nonadjacent_markers_and_duplicate_ids() {
        let body = concat!(
            "```md\n不要 ^code\n```\n\n",
            "別當識別碼 `^inline`\n\n",
            "別當識別碼 **^styled**\n\n",
            "[^linked](https://example.invalid/)\n\n",
            "保留原字元 \\^escaped\n\n",
            "> 引文\n^no-before\n\n",
            "> 引文\n\n^no-after\n# 直接接標題\n\n",
            "> 原引文\n\n***\n\n^not-adjacent\n\n",
            "重複一 ^twice\n\n- 重複二 ^twice\n",
        );
        for anchor in ["#^code", "#^inline", "#^styled", "#^linked", "#^escaped", "#^no-before", "#^no-after", "#^not-adjacent"] {
            assert_eq!(resolve_block(body, anchor), BlockLookup::Missing, "{anchor}");
        }
        assert_eq!(resolve_block(body, "#^twice"), BlockLookup::Ambiguous);
        for anchor in ["#^", "#^錯誤", "#^x_y", "#^id#different", "#%5Eid"] {
            assert_eq!(resolve_block(body, anchor), BlockLookup::Unsupported, "{anchor}");
        }
    }
}
