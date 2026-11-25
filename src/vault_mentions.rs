use super::{NATIVE_NOTE_PREFIX, Notebook, Result};
use crate::store::save_in;
use crate::store::vault_search::{MAX_SCAN, TagFilter, TagMatch, frontmatter, matches_tag, visit_property};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::ops::Range;

const MAX_RESULTS: usize = 200;
const MAX_NAMES: usize = 64;

#[derive(Clone, PartialEq, Eq)]
struct Source {
    id: i64,
    version: i64,
    import: i64,
    path: String,
    token: String,
}

struct SavedNote {
    source: Source,
    title: String,
    body: String,
}

fn read_note(row: &Row<'_>) -> rusqlite::Result<SavedNote> {
    Ok(SavedNote {
        source: Source {
            id: row.get(0)?,
            version: row.get(1)?,
            import: row.get(2)?,
            path: row.get(3)?,
            token: row.get(4)?,
        },
        title: row.get(5)?,
        body: row.get(6)?,
    })
}

const SELECT_NOTE: &str = "SELECT n.id,n.version,p.import_root,p.relative_path,i.token,n.title,n.body
    FROM vault_paths p JOIN notes n ON p.note_id=n.id
    JOIN notes root ON root.id=p.import_root JOIN note_link_identities i ON i.note_id=n.id
    WHERE n.trash_root IS NULL AND root.trash_root IS NULL";

fn saved_note(db: &Connection, id: i64) -> Result<Option<SavedNote>> {
    Ok(db.query_row(&format!("{SELECT_NOTE} AND n.id=?1"), [id], read_note).optional()?)
}

fn visit_names(note: &SavedNote, mut visit: impl FnMut(&str) -> Result<bool>) -> Result<()> {
    let basename = note.source.path.rsplit('/').next()
        .and_then(|name| name.rsplit_once('.'))
        .map_or("", |(base, _)| base);
    if visit(&note.title)? || visit(basename)? {
        return Ok(());
    }
    visit_property(frontmatter(&note.body).0, "aliases", visit)?;
    Ok(())
}

fn link_label(name: &str) -> bool {
    !name.is_empty() && name.len() <= 240
        && name.starts_with(char::is_alphanumeric) && name.ends_with(char::is_alphanumeric)
        && name.chars().all(|c| !c.is_control() && !matches!(c, '[' | ']' | '|' | '\\'))
}

fn unique_names(db: &Connection, target: &SavedNote) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    visit_names(target, |name| {
        if link_label(name) {
            names.insert(name.to_owned());
        }
        if names.len() > MAX_NAMES {
            return Err("這篇筆記可搜尋的名稱和別名超過 64 個。".into());
        }
        Ok(false)
    })?;
    if names.is_empty() {
        return Ok(names);
    }

    let bytes: i64 = db.query_row(
        "SELECT coalesce(sum(length(CAST(n.body AS BLOB))),0)
         FROM vault_paths p JOIN notes n ON n.id=p.note_id
         WHERE p.import_root=?1 AND n.trash_root IS NULL",
        [target.source.import],
        |row| row.get(0),
    )?;
    if bytes > MAX_SCAN as i64 {
        return Err("這批 Vault 的現用筆記超過 128 MiB，無法完整核對未連結提及。".into());
    }
    // Several case variants in the target still name the same note.
    let mut matching = names.iter().map(|name| name.to_lowercase()).collect::<BTreeSet<_>>();
    let mut stmt = db.prepare(&format!("{SELECT_NOTE} AND p.import_root=?1 AND n.id<>?2"))?;
    let mut rows = stmt.query((target.source.import, target.source.id))?;
    while let Some(row) = rows.next()? {
        visit_names(&read_note(row)?, |other| {
            matching.remove(&other.to_lowercase());
            Ok(matching.is_empty())
        })?;
        if matching.is_empty() {
            return Ok(BTreeSet::new());
        }
    }
    names.retain(|name| matching.contains(&name.to_lowercase()));
    Ok(names)
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-') || ('\u{0300}'..='\u{036f}').contains(&c)
}

fn plain_mention(prose: &str, range: &Range<usize>) -> bool {
    let before = &prose[..range.start];
    let after = &prose[range.end..];
    if before.chars().next_back().is_some_and(|c| word_char(c) || matches!(c, '#' | '\\' | '[' | '!'))
        || after.chars().next().is_some_and(|c| word_char(c) || c == ']')
    {
        return false;
    }

    // A plain URL, tag path or e-mail can remain a CommonMark text event.
    let token_left = before.char_indices().rev().take_while(|&(_, c)| !c.is_whitespace()).take(256).last();
    let token_right = after.char_indices().take_while(|&(_, c)| !c.is_whitespace()).nth(255);
    let left_token = token_left.map_or("", |(start, _)| &before[start..]);
    if token_right.is_some() || left_token.len() > 255 {
        return false;
    }
    let right_token = after.split(char::is_whitespace).next().unwrap_or("");
    !(left_token.contains("://") || right_token.contains("://")
        || left_token.contains('@') || right_token.contains('@')
        || left_token.contains('#') && left_token.ends_with('/')
        || left_token.trim_start_matches(['(', '<', '[', '"', '\'']).starts_with("www."))

}

fn mention_ranges(body: &str, names: &BTreeSet<String>, mut visit: impl FnMut(Range<usize>) -> bool) {
    let (_, prose) = frontmatter(body);
    if prose == body.strip_prefix('\u{feff}').unwrap_or(body)
        && (prose.starts_with("---\n") || prose.starts_with("---\r\n"))
    {
        // A metadata opener without a closing delimiter does not provide a safe prose boundary.
        return;
    }
    let offset = body.len() - prose.len();
    let mut hidden = 0usize;
    for (event, range) in Parser::new_ext(prose, Options::ENABLE_WIKILINKS).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { .. } | Tag::Image { .. } | Tag::CodeBlock(_) | Tag::HtmlBlock) => hidden += 1,
            Event::End(TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock | TagEnd::HtmlBlock) => hidden -= 1,
            Event::Text(value) if hidden == 0 && value.as_ref() == &prose[range.clone()] => {
                let mut candidates = names.iter().map(|name| {
                    value.match_indices(name).map(|(local, matched)| {
                        let start = range.start + local;
                        start..start + matched.len()
                    }).peekable()
                }).collect::<Vec<_>>();
                let mut previous_end = range.start;
                loop {
                    let next = candidates.iter_mut().enumerate()
                        .filter_map(|(i, iter)| iter.peek().map(|span| (i, span.start, Reverse(span.end))))
                        .min_by_key(|&(_, start, end)| (start, end));
                    let Some((i, _, _)) = next else { break; };
                    let Some(span) = candidates[i].next() else { break; };
                    if span.start >= previous_end && plain_mention(prose, &span) {
                        previous_end = span.end;
                        if visit(span.start + offset..span.end + offset) {
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub(crate) struct VaultMention {
    target: Source,
    source: Source,
    span: Range<usize>,
    line: usize,
    spelling: String,
    context: String,
}

fn nearby(body: &str, range: &Range<usize>) -> (usize, String) {
    let before = &body[..range.start];
    let line = 1 + before.bytes().filter(|b| *b == b'\n').count();
    let left = before.rsplit('\n').next().unwrap_or("");
    let left = left.char_indices().rev().nth(139).map_or(left, |(start, _)| &left[start..]);
    let right: String = body[range.end..].split(['\r', '\n']).next().unwrap_or("").chars().take(140).collect();
    (line, format!("{left}{}{right}", &body[range.clone()]))
}

impl VaultMention {
    pub(crate) fn label(&self) -> String {
        format!("{}　第 {} 行　{}", self.source.path, self.line, self.spelling)
    }

    pub(crate) fn context(&self) -> String {
        format!("來源　{}（第 {} 行）\n目標　{}\n提及　{}\n\n{}\n\n建立連結只會更動紙間內這一處，來源 Markdown 原檔保留。可用筆記版本記錄復原。",
            self.source.path, self.line, self.target.path, self.spelling, self.context)
    }
}

impl Notebook {
    pub(crate) fn vault_unlinked_mentions(&self, target: i64, source_tag: &str) -> Result<(Vec<VaultMention>, bool)> {
        let tx = self.connection.unchecked_transaction()?;
        let target = saved_note(&tx, target)?.ok_or("請先開啟要找提及的 Vault 匯入筆記。")?;
        let tag = (!source_tag.trim().is_empty())
            .then(|| TagFilter::parse(source_tag, TagMatch::Exact)).transpose()?;
        let names = unique_names(&tx, &target)?;
        let mut hits = Vec::new();
        let mut limited = false;
        if !names.is_empty() {
            let mut stmt = tx.prepare(&format!(
                "{SELECT_NOTE} AND p.import_root=?1 AND n.id<>?2 ORDER BY p.relative_path,n.id",
            ))?;
            for source in stmt.query_map((target.source.import, target.source.id), read_note)? {
                let source = source?;
                if let Some(tag) = &tag && !matches_tag(&source.body, tag)? {
                    continue;
                }
                mention_ranges(&source.body, &names, |span| {
                    if hits.len() == MAX_RESULTS {
                        limited = true;
                        return true;
                    }
                    let (line, context) = nearby(&source.body, &span);
                    hits.push(VaultMention {
                        target: target.source.clone(),
                        source: source.source.clone(),
                        spelling: source.body[span.clone()].to_owned(),
                        line,
                        context,
                        span,
                    });
                    false
                });
                if limited {
                    break;
                }
            }
        }
        tx.commit()?;
        Ok((hits, limited))
    }

    pub(crate) fn link_vault_mention(&self, hit: &VaultMention) -> Result<i64> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let stale = "這處提及、目標名稱或來源路徑已變更。請重新檢查，尚未建立連結。";
        let target = saved_note(&tx, hit.target.id)?.filter(|note| note.source == hit.target).ok_or(stale)?;
        let mut source = saved_note(&tx, hit.source.id)?.filter(|note| note.source == hit.source).ok_or(stale)?;
        let names = unique_names(&tx, &target)?;
        let mut eligible = false;
        mention_ranges(&source.body, &names, |span| {
            eligible = span == hit.span && source.body[span.clone()] == hit.spelling;
            eligible || span.start > hit.span.start
        });
        if !eligible {
            return Err(stale.into());
        }
        let destination = format!("{NATIVE_NOTE_PREFIX}{}:{}", target.source.id, target.source.token);
        let link = format!("[[{destination}|{}]]", hit.spelling);
        source.body.replace_range(hit.span.clone(), &link);
        if !crate::markdown::note_references(&source.body).iter().any(|reference| {
            reference.offset == hit.span.start && reference.destination == destination && reference.label == hit.spelling
        }) {
            return Err("這處 Markdown 無法保留原文並建立有效連結，未變更筆記。".into());
        }
        save_in(&tx, source.source.id, source.source.version, &source.title, &source.body)?;
        tx.commit()?;
        Ok(source.source.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_mentions_link_one_unique_alias_and_restore_original_after_offline_reopen() -> Result<()> {
        use std::fs;
        let dir = crate::store::tests::data_dir();
        let vault = dir.join("vault");
        let offline = dir.join("offline");
        let sqlite = dir.join("notes.sqlite");
        for folder in ["甲", "乙", "現場"] {
            fs::create_dir_all(vault.join(folder))?;
        }
        let entry_path = "現場/入口.md";
        let entry_body = concat!(
            "\u{feff}---\r\ntags: [拍攝/夜景]\r\n摘要: 鏡頭專名\r\n---\r\n",
            "# 入口\r\n\r\n🎬 先看 鏡頭專名，謝謝。\r\n",
            "\r\n子鏡頭專名 鏡頭專名後 鏡頭 鏡頭-專名 #鏡頭專名 #別類/鏡頭專名 重擿名 CaseName\r\n",
            "\r\n[[乙/鏡頭|鏡頭專名]] [鏡頭專名](乙/鏡頭.md) `鏡頭專名` https://invalid.example/鏡頭專名\r\n",
            "\r\n```text\r\n鏡頭專名\r\n```\r\n",
        );
        let target_path = "乙/鏡頭.md";
        let target_body = "---\naliases: [鏡頭專名, 重擿名, CaseName]\n---\n目標本文";
        fs::write(vault.join(target_path), target_body)?;
        fs::write(vault.join("甲/鏡頭.md"), "---\naliases:\n  - 重擿名\n  - casename\n---\n另一篇同名筆記")?;
        fs::write(vault.join(entry_path), entry_body)?;
        fs::write(vault.join("現場/其他.md"), "#拍攝/夜景/夜班\n\n另一處 鏡頭專名")?;
        let mut db = Notebook::open(&sqlite, false)?;
        let root = db.import_vault("拍攝合成樣本", crate::vault::read(&vault)?.pages)?;
        db.import_vault("另一匯入批次", crate::vault::read(&vault)?.pages)?;
        let id = |path: &str| -> Result<i64> {
            Ok(db.connection.query_row(
                "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
                (root, path),
                |r| r.get(0),
            )?)
        };
        let target = id(target_path)?;
        let entry = id(entry_path)?;
        assert_eq!(db.vault_unlinked_mentions(target, "")?.0.len(), 2);
        assert!(db.vault_unlinked_mentions(target, "#拍攝")?.0.is_empty());
        let mut hits = db.vault_unlinked_mentions(target, "#拍攝/夜景")?.0;
        assert_eq!(hits.len(), 1);
        let hit = hits.pop().unwrap();
        assert_eq!(hit.label(), "現場/入口.md　第 7 行　鏡頭專名");
        assert!(hit.context().contains("🎬 先看 鏡頭專名，謝謝。"));
        assert_eq!(db.note(entry)?.unwrap().body, entry_body);
        assert!(db.note_revisions(entry)?.is_empty());
        assert_eq!(db.link_vault_mention(&hit)?, entry);
        assert!(db.link_vault_mention(&hit).is_err());
        assert!(db.vault_unlinked_mentions(target, "#拍攝/夜景")?.0.is_empty());

        fs::rename(&vault, &offline)?;
        drop(db);
        let db = Notebook::open(&sqlite, false)?;
        let saved = db.note(entry)?.unwrap();
        let link = format!("[[{NATIVE_NOTE_PREFIX}{target}:{}|鏡頭專名]]", hit.target.token);
        let expected = entry_body.replacen("🎬 先看 鏡頭專名", &format!("🎬 先看 {link}"), 1);
        assert_eq!(saved.body, expected);
        let links = db.vault_note_links(&saved)?;
        assert!(links.iter().any(|l| l.note_id == target && l.label == "鏡頭專名" && l.anchor.is_none()));
        let revisions = db.note_revisions(entry)?;
        assert_eq!(revisions.len(), 1);
        assert_eq!(db.note_revision(entry, revisions[0].id)?.1, entry_body);
        db.restore_note_revision(entry, saved.version, revisions[0].id, None)?;
        assert_eq!(db.note(entry)?.unwrap().body, entry_body);
        assert_eq!(fs::read(offline.join(entry_path))?, entry_body.as_bytes());
        assert_eq!(fs::read(offline.join(target_path))?, target_body.as_bytes());
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn vault_mention_confirmation_rechecks_a_third_note_alias_and_source_version() -> Result<()> {
        use crate::archive::ImportedPage;
        let dir = crate::store::tests::data_dir();
        let sqlite = dir.join("notes.sqlite");
        let mut db = Notebook::open(&sqlite, false)?;
        let make = |path: &str, body: &str| ImportedPage {
            parts: vec![path.into()],
            title: path.trim_end_matches(".md").into(),
            body: body.into(),
        };
        let root = db.import_vault("V", vec![
            make("甲.md", "---\naliases: [顯示別名]\n---\n目標"),
            make("乙.md", "有 顯示別名"),
            make("丙.md", "原本沒有同名別名"),
        ])?;
        let id = |path: &str| -> Result<i64> {
            Ok(db.connection.query_row(
                "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
                (root, path), |r| r.get(0),
            )?)
        };
        let (target, entry, third) = (id("甲.md")?, id("乙.md")?, id("丙.md")?);
        let hit = db.vault_unlinked_mentions(target, "")?.0.pop().unwrap();
        let other = Notebook::open(&sqlite, false)?;
        let note = other.note(third)?.unwrap();
        let version = other.save(third, note.version, &note.row.title, "---\naliases: [顯示別名]\n---\n撞名")?;
        assert!(db.link_vault_mention(&hit).is_err());
        assert_eq!(db.note(entry)?.unwrap().body, "有 顯示別名");
        assert!(db.note_revisions(entry)?.is_empty());

        other.save(third, version, &note.row.title, &note.body)?;
        let hit = db.vault_unlinked_mentions(target, "")?.0.pop().unwrap();
        let note = other.note(entry)?.unwrap();
        other.save(entry, note.version, &note.row.title, "改成 顯示別名")?;
        assert!(db.link_vault_mention(&hit).is_err());
        assert_eq!(db.note(entry)?.unwrap().body, "改成 顯示別名");
        assert_eq!(db.note_revisions(entry)?.len(), 1);
        drop(other);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
