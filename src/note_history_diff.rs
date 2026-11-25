use std::collections::HashMap;
use std::fmt::Write;

const MAX_LINES: usize = 16_000;
const MAX_CELLS: usize = 1_000_000;
const MAX_SHOWN: usize = 60;
const MAX_CHARS: usize = 100;

fn lines(mut text: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if text.is_empty() { return None; }
        let end = text.find(['\r', '\n']).map_or(text.len(), |i| {
            i + if text[i..].starts_with("\r\n") { 2 } else { 1 }
        });
        let (line, rest) = text.split_at(end);
        text = rest;
        Some(line)
    })
}

fn preview_line(text: &str) -> String {
    let (content, ending) = if let Some(s) = text.strip_suffix("\r\n") { (s, "CRLF") }
        else if let Some(s) = text.strip_suffix('\r') { (s, "CR") }
        else if let Some(s) = text.strip_suffix('\n') { (s, "LF") }
        else { (text, "無換行") };
    let mut chars = content.chars();
    let mut visible: String = chars.by_ref().take(MAX_CHARS)
        .map(|c| if c == '\t' { '⇥' } else { c }).collect();
    if chars.next().is_some() { visible.push('…'); }
    format!("{visible}  ⟦{ending}⟧")
}

pub(crate) fn comparison(current_title: &str, current: &str, title: &str, target: &str) -> String {
    let mut text = String::new();
    if current_title == title {
        let _ = writeln!(text, "標題相同　{title}");
    } else {
        let _ = writeln!(text, "− 目前標題　{current_title}\n＋ 舊版標題　{title}");
    }
    if current == target {
        text.push_str("\n正文相同。");
        return text;
    }
    if target.is_empty() {
        text.push_str("\n選取的舊版沒有正文。\n");
    }
    let _ = writeln!(text, "\n− 目前正文　＋ 舊版正文　（數字為各自的行號）");
    let old: Vec<_> = lines(current).take(MAX_LINES + 1).collect();
    let new: Vec<_> = lines(target).take(MAX_LINES + 1).collect();
    let start = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    if old.len() > MAX_LINES || new.len() > MAX_LINES {
        let _ = writeln!(text, "正文差異超過預覽的行數上限。已確認保存的兩份原文不同。");
        return text;
    }
    let end = old[start..].iter().rev().zip(new[start..].iter().rev())
        .take_while(|(a, b)| a == b).count();
    let old = &old[start..old.len()-end];
    let new = &new[start..new.len()-end];
    let width = new.len() + 1;
    let cells = (old.len() + 1) * width;
    let mut table = Vec::new();
    if cells <= MAX_CELLS {
        let mut ids = HashMap::new();
        let mut intern = |line| {
            let id = ids.len();
            *ids.entry(line).or_insert(id)
        };
        let indexed_old: Vec<_> = old.iter().copied().map(&mut intern).collect();
        let indexed_new: Vec<_> = new.iter().copied().map(&mut intern).collect();
        table.resize(cells, 0u16);
        for i in (0..old.len()).rev() {
            for j in (0..new.len()).rev() {
                let at = i * width + j;
                table[at] = if indexed_old[i] == indexed_new[j] {
                    1 + table[at + width + 1]
                } else {
                    table[at + width].max(table[at + 1])
                };
            }
        }
    } else {
        text.push_str("差異範圍較長，下列以兩版的變動範圍呈現。\n");
    }
    text.push_str("長行以省略號截短，換行類型標在行末。\n");
    let (mut i, mut j, mut shown) = (0, 0, 0);
    while i < old.len() || j < new.len() {
        if !table.is_empty() && i < old.len() && j < new.len() && old[i] == new[j] {
            i += 1;
            j += 1;
            continue;
        }
        if shown == MAX_SHOWN {
            text.push_str("（其餘差異已省略，還原會使用完整舊版。）\n");
            break;
        }
        let remove = i < old.len() && (j == new.len() || if table.is_empty() {
            i <= j
        } else {
            table[(i + 1) * width + j] >= table[i * width + j + 1]
        });
        if remove {
            let _ = writeln!(text, "− {:>5}  {}", start + i + 1, preview_line(old[i]));
            i += 1;
        } else {
            let _ = writeln!(text, "＋ {:>5}  {}", start + j + 1, preview_line(new[j]));
            j += 1;
        }
        shown += 1;
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_tracks_repeated_lines_blanks_and_raw_endings() {
        let diff = comparison("新🎬", "重複\r\n\r\n新段\r\n重複\n", "舊🎬", "重複\r\n\n舊段\n重複\n");
        assert!(diff.contains("− 目前標題　新🎬") && diff.contains("＋ 舊版標題　舊🎬"));
        assert!(diff.contains("−     2    ⟦CRLF⟧") && diff.contains("＋     2    ⟦LF⟧"));
        assert!(diff.contains("−     3  新段") && diff.contains("＋     3  舊段"));
        assert!(!diff.contains("重複"));
        assert!(comparison("同", "內文", "同", "").contains("沒有正文"));
        assert!(comparison("同", "", "同", "").contains("正文相同"));
    }

    #[test]
    fn comparison_bounds_disjoint_and_overlong_input() {
        let preview = comparison("同", &"甲\n".repeat(1500), "同", &"乙\n".repeat(1500));
        assert!(preview.contains("差異範圍較長") && preview.contains("其餘差異已省略"));
        assert_eq!(preview.lines().filter(|s| s.starts_with("− ") || s.starts_with("＋ ")).count(), 61);
        assert!(comparison("同", &"\n".repeat(MAX_LINES + 1), "同", "")
            .contains("超過預覽的行數上限"));
    }
}
