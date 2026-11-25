//! Find original note text. The index only supplies row IDs. All visibility,
//! filters, and byte positions are derived from the current workspace rows.
use super::navigation::{BlockNoteSnapshot, BlockNoteState, SourceSelection, editor_source_selection};
use super::vault_search::{MAX_SCAN, TagFilter, TagMatch, matches_tag, source_folder};
use super::{Notebook, Result};
use rusqlite::params;
use std::collections::BTreeSet;
use std::ops::Range;

pub(crate) const MAX_QUERY_CHARS: usize = 120;
pub(crate) const RESULT_LIMIT: usize = 100;

pub(super) const SCHEMA: &str = "
CREATE VIRTUAL TABLE workspace_note_search USING fts5(
    title, body, tokenize='trigram case_sensitive 1',
    detail=none, content='', contentless_delete=1
);
INSERT INTO workspace_note_search(rowid,title,body)
    SELECT id,lower(title),lower(body) FROM notes WHERE trash_root IS NULL;
CREATE TRIGGER search_note_insert AFTER INSERT ON notes
WHEN new.trash_root IS NULL
BEGIN
    INSERT INTO workspace_note_search(rowid,title,body) VALUES(new.id,lower(new.title),lower(new.body));
END;
CREATE TRIGGER search_note_change AFTER UPDATE OF title,body,trash_root ON notes
WHEN old.title<>new.title OR old.body<>new.body OR old.trash_root IS NOT new.trash_root
BEGIN
    DELETE FROM workspace_note_search WHERE rowid=old.id AND old.trash_root IS NULL;
    INSERT INTO workspace_note_search(rowid,title,body)
        SELECT new.id,lower(new.title),lower(new.body) WHERE new.trash_root IS NULL;
END;
CREATE TRIGGER search_note_delete AFTER DELETE ON notes
WHEN old.trash_root IS NULL
BEGIN
    DELETE FROM workspace_note_search WHERE rowid=old.id;
END;";

#[derive(Clone, Copy)]
pub(crate) enum SearchPosition {
    Title { start: usize, end: usize },
    Body { start: usize, end: usize, line: usize },
    Note,
}

pub(crate) struct SearchHit {
    pub snapshot: BlockNoteSnapshot,
    identity: String,
    pub title: String,
    pub path: Option<String>,
    pub context: String,
    pub position: SearchPosition,
}

pub(crate) struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub limited: bool,
}

fn trigrams(query: &str) -> Option<String> {
    let chars = query.chars().collect::<Vec<_>>();
    if chars.len() < 3 { return None; }
    // FTS5 detail=none supports three-character tokens. Requiring the
    // distinct grams narrows candidates while keeping positional data out
    // of the index. Match the literal in the live text after this step.
    let grams = chars.windows(3).map(|window| window.iter().collect::<String>())
        .collect::<BTreeSet<_>>();
    Some(grams.into_iter().map(|gram| format!("\"{}\"", gram.replace('"', "\"\"")))
        .collect::<Vec<_>>().join(" AND "))
}

fn literal_match(text: &str, normalized: &str) -> Option<Range<usize>> {
    if normalized.is_empty() { return None; }
    let start = if normalized.bytes().any(|byte| byte.is_ascii_alphabetic()) {
        text.to_ascii_lowercase().find(normalized)?
    } else {
        text.find(normalized)?
    };
    Some(start..start + normalized.len())
}

fn context(text: &str, range: Range<usize>) -> String {
    let start = text[..range.start].char_indices().rev().nth(41).map_or(0, |(i, _)| i);
    let end = text[range.end..].char_indices().nth(58).map_or(text.len(), |(i, _)| range.end + i);
    let before = if start == 0 { "" } else { "…" };
    let after = if end == text.len() { "" } else { "…" };
    let display = |segment: &str| segment.chars().map(|c| {
        if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { ' ' } else { c }
    }).collect::<String>();
    let hit = if range.is_empty() { String::new() } else { format!("〔{}〕", display(&text[range.clone()])) };
    format!("{before}{}{hit}{}{after}", display(&text[start..range.start]), display(&text[range.end..end]))
}

impl Notebook {
    pub(crate) fn search_hit_is_current(&self, hit: &SearchHit) -> Result<bool> {
        let tx = self.connection.unchecked_transaction()?;
        let same_identity = tx.prepare_cached(
            "SELECT 1 FROM note_link_identities WHERE note_id=?1 AND token=?2",
        )?.exists(params![hit.snapshot.id, hit.identity])?;
        let note = self.note(hit.snapshot.id)?;
        tx.commit()?;
        Ok(same_identity && hit.snapshot.state(note.as_ref()) == BlockNoteState::Unchanged)
    }

    pub(crate) fn search_workspace(&self, text: &str, tag: &str, folder: &str) -> Result<SearchResults> {
        let text = text.trim();
        if text.chars().count() > MAX_QUERY_CHARS || text.chars().any(char::is_control) {
            return Err(format!("關鍵字最多 {MAX_QUERY_CHARS} 個字元，且不能含控制字元。").into());
        }
        let folder = source_folder(folder)?;
        let tag = tag.trim();
        let tag_filter = if tag.is_empty() {
            None
        } else if let Some(exact) = tag.strip_prefix('=') {
            Some(TagFilter::parse(exact, TagMatch::Exact)?)
        } else {
            Some(TagFilter::parse(tag, TagMatch::Descendants)?)
        };
        if text.is_empty() && tag_filter.is_none() && folder.is_empty() {
            return Err("請輸入關鍵字、標籤或來源資料夾。".into());
        }

        let needle = text.to_ascii_lowercase();
        let pattern = trigrams(&needle);
        let (source, filter, order, input) = match &pattern {
            Some(pattern) => (
                "workspace_note_search CROSS JOIN notes n ON n.id=workspace_note_search.rowid",
                "workspace_note_search MATCH ?1", "workspace_note_search.rowid", pattern.as_str(),
            ),
            None => (
                "notes n", "(?1='' OR instr(lower(n.title),?1)>0 OR instr(lower(n.body),?1)>0)",
                "n.id", needle.as_str(),
            ),
        };
        let sql = format!(
            "SELECT n.id,n.title,n.body,n.version,p.relative_path,i.token
             FROM {source}
             JOIN note_link_identities i ON i.note_id=n.id
             LEFT JOIN vault_paths p ON p.note_id=n.id AND EXISTS (
                 SELECT 1 FROM notes root WHERE root.id=p.import_root AND root.trash_root IS NULL
             )
             WHERE n.trash_root IS NULL AND {filter}
             AND (?2='' OR instr(p.relative_path,?2||'/')=1)
             ORDER BY {order} DESC"
        );
        let mut stmt = self.connection.prepare(&sql)?;
        let mut rows = stmt.query(params![input, folder])?;
        let mut result = SearchResults { hits: Vec::new(), limited: false };
        let mut scanned = 0;
        while let Some(row) = rows.next()? {
            let title = row.get_ref(1)?.as_str()?;
            let body = row.get_ref(2)?.as_str()?;
            scanned += body.len();
            if scanned > MAX_SCAN {
                result.limited = true;
                break;
            }
            let (position, snippet) = if let Some(range) = literal_match(body, &needle) {
                let SourceSelection { start, end, line } = editor_source_selection(body, range.clone())
                    .ok_or("無法核對正文的搜尋位置。")?;
                (SearchPosition::Body { start, end, line }, context(body, range))
            } else if let Some(range) = literal_match(title, &needle) {
                let start = title[..range.start].encode_utf16().count();
                let end = start + title[range.clone()].encode_utf16().count();
                (SearchPosition::Title { start, end }, context(title, range))
            } else if needle.is_empty() {
                (SearchPosition::Note, context(body.trim_start(), 0..0))
            } else {
                continue;
            };
            if let Some(filter) = &tag_filter && !matches_tag(body, filter)? { continue; }
            if result.hits.len() == RESULT_LIMIT {
                result.limited = true;
                break;
            }
            result.hits.push(SearchHit {
                snapshot: BlockNoteSnapshot::from_parts(row.get(0)?, row.get(3)?, title, body),
                identity: row.get(5)?,
                title: title.to_owned(),
                path: row.get(4)?,
                position,
                context: snippet,
            });
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use crate::store::navigation::BlockNoteState;

    #[test]
    fn upgrades_searches_reopens_and_keeps_trashed_notes_out_even_with_a_stale_index_entry() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let body = "---\r\ntags: [製片/早班]\r\n---\r\n📝 左側 訊號🧪 右側";
        let root = db.import_vault("合成來源", vec![
            ImportedPage { parts: vec!["製片".into(), "同名.md".into()], title: "同名".into(), body: body.into() },
            ImportedPage { parts: vec!["製片舊".into(), "同名.md".into()], title: "同名".into(), body: "訊號🧪 另一版 #製片/早班".into() },
        ])?;
        let old = db.add(None, "引號測試：A\"B", "拆分 abc bcd")?;
        super::super::tests::remove_v28_schema(&db.connection)?;
        db.connection.pragma_update(None, "user_version", 28)?;
        drop(db);

        let mut db = Notebook::open(&path, false)?;
        let within = db.search_workspace("訊號🧪", "#製片", "製片")?;
        assert_eq!(within.hits.len(), 1);
        let hit = &within.hits[0];
        assert!(db.search_hit_is_current(hit)?);
        assert_eq!(hit.path.as_deref(), Some("製片/同名.md"));
        assert!(hit.context.contains("左側 〔訊號🧪〕 右側"));
        let start = body.split_once("訊號🧪").unwrap().0.encode_utf16().count();
        assert!(matches!(hit.position, SearchPosition::Body { start: first, end, line: 4 }
            if first == start && end == start + "訊號🧪".encode_utf16().count()));
        let id = hit.snapshot.id;
        let snapshot = hit.snapshot;
        assert_eq!(db.search_workspace("訊號", "", "")?.hits.len(), 2);
        assert!(db.search_workspace("訊號🧪", "=#製片", "")?.hits.is_empty());
        assert_eq!(db.search_workspace("", "=#製片/早班", "製片")?.hits.len(), 1);
        assert!(db.search_workspace("abcd", "", "")?.hits.is_empty());
        let quoted = db.search_workspace("a\"b", "", "")?;
        assert_eq!(quoted.hits.len(), 1);
        assert_eq!(quoted.hits[0].snapshot.id, old);
        assert!(matches!(quoted.hits[0].position, SearchPosition::Title { .. }));
        for (query, tag, folder) in [("", "", ""), ("x", "", "../製片"), ("x", "#製片//舊", "")] {
            assert!(db.search_workspace(query, tag, folder).is_err());
        }

        db.trash(root)?;
        assert!(!db.search_hit_is_current(hit)?);
        assert_eq!(snapshot.state(db.note(id)?.as_ref()), BlockNoteState::Unavailable);
        db.connection.execute("INSERT INTO workspace_note_search(rowid,title,body) VALUES(?1,'同名',?2)",
            params![id, body])?;
        assert!(db.search_workspace("訊號🧪", "#製片", "")?.hits.is_empty());
        db.connection.execute("DELETE FROM workspace_note_search WHERE rowid=?1", [id])?;
        drop(db);
        let mut db = Notebook::open(&path, false)?;
        assert!(db.search_workspace("訊號", "", "")?.hits.is_empty());
        db.restore(root)?;
        assert_eq!(db.search_workspace("訊號🧪", "#製片", "")?.hits.len(), 2);
        let ver = db.note(id)?.unwrap().version;
        db.save(id, ver, "同名", "新版本只有 替代🧪")?;
        assert_eq!(snapshot.state(db.note(id)?.as_ref()), BlockNoteState::Revised);
        assert!(!db.search_hit_is_current(hit)?);
        assert!(db.search_workspace("訊號🧪", "", "製片")?.hits.is_empty());
        assert_eq!(db.search_workspace("替代🧪", "", "製片")?.hits[0].snapshot.id, id);
        db.connection.execute("DELETE FROM notes WHERE id=?1", [old])?;
        assert!(db.search_workspace("a\"b", "", "")?.hits.is_empty());
        let replacement = db.add(None, "引號測試：A\"B", "拆分 abc bcd")?;
        assert_eq!(replacement, old);
        assert!(!db.search_hit_is_current(&quoted.hits[0])?);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn large_workspace_returns_only_bounded_distinct_live_notes_in_stable_order() -> Result<()> {
        let db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        db.connection.execute_batch(
            "WITH RECURSIVE items(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM items WHERE i<1600)
             INSERT INTO notes(id,title,body)
             SELECT i,'板卡 '||i,CASE WHEN i<=200 THEN '合成目標 合成目標 #現場' ELSE '另一場戲' END
             FROM items;",
        )?;
        let results = db.search_workspace("合成目標", "#現場", "")?;
        assert!(results.limited);
        assert_eq!(results.hits.len(), RESULT_LIMIT);
        assert_eq!(results.hits.first().unwrap().snapshot.id, 200);
        assert_eq!(results.hits.last().unwrap().snapshot.id, 101);
        assert!(db.search_workspace(&"x".repeat(MAX_QUERY_CHARS + 1), "", "").is_err());
        Ok(())
    }
}
