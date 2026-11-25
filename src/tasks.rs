//! Project Markdown task markers from saved notes without keeping a second task database.
use super::{Notebook, Note, Result, navigation::SourceSelection, save_in};
use pulldown_cmark::{Event, Options, Parser};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::ops::Range;

pub(crate) const TASK_LIMIT: usize = 200;

#[derive(Clone, Copy)]
pub(crate) enum TaskScope {
    Workspace,
    Board(i64),
    Note(i64),
}

impl TaskScope {
    fn filters(self) -> (Option<i64>, Option<i64>) {
        match self {
            Self::Workspace => (None, None),
            Self::Board(id) => (Some(id), None),
            Self::Note(id) => (None, Some(id)),
        }
    }
}

pub(crate) struct TaskHit {
    pub note: i64,
    pub done: bool,
    pub label: String,
    identity: String,
    revision: i64,
    marker: Range<usize>,
    scope: TaskScope,
}

pub(super) fn task_markers(body: &str) -> impl Iterator<Item = (Range<usize>, bool)> + '_ {
    let (_, prose) = super::vault_search::frontmatter(body);
    let base = body.len() - prose.len();
    let options = Options::ENABLE_TASKLISTS | Options::ENABLE_GFM | Options::ENABLE_WIKILINKS;
    Parser::new_ext(prose, options).into_offset_iter().filter_map(move |(event, span)| {
        let Event::TaskListMarker(done) = event else { return None };
        let marker = prose.get(span.clone())?;
        let offset = marker.find('[')?;
        let start = span.start + offset;
        let raw = prose.as_bytes().get(start..start+3)?;
        let valid = match raw {
            [b'[', b'x' | b'X', b']'] => done,
            [b'[', b' ' | b'\t', b']'] => !done,
            _ => false,
        };
        valid.then_some((base+start..base+start+3, done))
    })
}

fn single_line(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut label = chars.by_ref().take(limit).map(|ch| if ch.is_control() { ' ' } else { ch }).collect::<String>();
    if chars.next().is_some() { label.push('…'); }
    label
}

impl Notebook {
    pub(crate) fn task_overview(
        &self, scope: TaskScope, done: Option<bool>, query: &str,
    ) -> Result<(Vec<TaskHit>, bool)> {
        let query = query.trim();
        if query.len() > 4096 || query.chars().any(char::is_control) {
            return Err("待辦查詢最多 4,096 位元組，請使用單行文字。".into());
        }
        let query = query.to_lowercase();
        let (board, note) = scope.filters();
        let mut statement = self.connection.prepare(
            "SELECT n.id,n.title,n.body,n.version,i.token,p.relative_path
             FROM notes n JOIN note_link_identities i ON i.note_id=n.id
             LEFT JOIN vault_paths p ON p.note_id=n.id
             WHERE n.trash_root IS NULL AND (?1 IS NULL OR n.id=?1) AND instr(n.body,'[')>0
               AND (?2 IS NULL OR EXISTS(SELECT 1 FROM board_cards c WHERE c.board_id=?2 AND c.note_id=n.id))
             ORDER BY n.id",
        )?;
        let mut rows = statement.query(params![note, board])?;
        let mut hits = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let id = row.get(0)?;
            let title = row.get_ref(1)?.as_str()?;
            let body = row.get_ref(2)?.as_str()?;
            let revision = row.get(3)?;
            let identity = row.get_ref(4)?.as_str()?;
            let path = row.get_ref(5)?.as_str_or_null()?;
            bytes += body.len();
            if bytes > super::vault_search::MAX_SCAN {
                return Ok((hits, true));
            }
            let source_match = query.is_empty() || title.to_lowercase().contains(&query)
                || path.is_some_and(|path| path.to_lowercase().contains(&query));
            let mut line = 1;
            let mut previous = 0;
            for (marker, complete) in task_markers(body) {
                let prefix = &body[previous..marker.start];
                line += prefix.matches(['\r', '\n']).count() - prefix.matches("\r\n").count();
                previous = marker.start;
                if done.is_some_and(|done| complete != done) { continue; }
                let text = body[marker.end..].split(['\r', '\n']).next().unwrap_or("").trim();
                if !source_match && !text.to_lowercase().contains(&query) { continue; }
                if hits.len() == TASK_LIMIT { return Ok((hits, true)); }
                let source = single_line(path.unwrap_or(title), 58);
                let excerpt = single_line(text, 76);
                let state = if complete { "[x]" } else { "[ ]" };
                let label = format!("{source} [#{id}] 第 {line} 行　{state} {excerpt}");
                hits.push(TaskHit { note: id, done: complete, label, identity: identity.into(), revision, marker, scope });
            }
        }
        Ok((hits, false))
    }

    fn current_task(&self, hit: &TaskHit) -> Result<Note> {
        let (board, _) = hit.scope.filters();
        let active = self.connection.prepare_cached(
            "SELECT 1 FROM notes n JOIN note_link_identities i ON i.note_id=n.id
             WHERE n.id=?1 AND n.version=?2 AND i.token=?3 AND n.trash_root IS NULL
             AND (?4 IS NULL OR EXISTS(SELECT 1 FROM board_cards c WHERE c.board_id=?4 AND c.note_id=n.id))",
        )?.exists(params![hit.note, hit.revision, hit.identity, board])?;
        if !active {
            return Err("待辦或所在白板已變更，請重新搜尋。".into());
        }
        let note = self.note(hit.note)?.ok_or("找不到待辦來源。")?;
        if !task_markers(&note.body).any(|(span, done)| span == hit.marker && done == hit.done) {
            return Err("待辦的位置或勾選狀態已變更，請重新搜尋。".into());
        }
        Ok(note)
    }

    pub(crate) fn task_position(&self, hit: &TaskHit) -> Result<(i64, SourceSelection)> {
        let tx = self.connection.unchecked_transaction()?;
        let note = self.current_task(hit)?;
        let position = super::navigation::editor_source_selection(&note.body, hit.marker.clone())
            .ok_or("無法定位待辦原文。")?;
        tx.commit()?;
        Ok((note.version, position))
    }

    pub(crate) fn toggle_listed_task(&self, hit: &TaskHit) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let mut note = self.current_task(hit)?;
        note.body.replace_range(hit.marker.start+1..hit.marker.start+2, if hit.done { " " } else { "x" });
        save_in(&tx, hit.note, note.version, &note.row.title, &note.body)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use crate::store::DEFAULT_BOARD_ID;

    #[test]
    fn task_overview_filters_toggles_exact_source_and_reopens_offline() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("tasks.sqlite3");
        let (first, second, trashed) = {
            let mut db = Notebook::open(&path, false)?;
            let root = db.import_vault("測試匯入", vec![
                ImportedPage { title: "鏡頭".into(), parts: vec!["甲".into(), "鏡頭.md".into()],
                    body: "---\r\n- [ ] 隱藏屬性\r\n---\r\n\r\n🎬 段落\r\n\r\n- [ ] 確認分鏡\r\n  續行\r\n  - [ ] 巢狀待辦\r\n\r\n```\r\n- [ ] 程式碼\r\n```\r\n".into() },
                ImportedPage { title: "鏡頭".into(), parts: vec!["乙".into(), "鏡頭.md".into()],
                    body: "1. [X] 確認分鏡\n\n- [ ] 確認分鏡\n\n- [ ] 另一項目\n".into() },
            ])?;
            let lookup = |folder: &str| db.connection.query_row(
                "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
                params![root, format!("{folder}/鏡頭.md")], |row| row.get::<_, i64>(0),
            );
            let first = lookup("甲")?;
            let second = lookup("乙")?;
            let trashed = db.add(None, "垃圾項目", "- [ ] 確認分鏡")?;
            db.add_card_on(DEFAULT_BOARD_ID, first)?;
            db.trash(trashed)?;
            (first, second, trashed)
        };
        {
            let db = Notebook::open(&path, false)?;
            let (all, truncated) = db.task_overview(TaskScope::Workspace, Some(false), "確認分鏡")?;
            assert!(!truncated);
            assert_eq!(all.iter().map(|hit| hit.note).collect::<Vec<_>>(), [first, second]);
            assert!(all[0].label.contains("甲/鏡頭.md") && all[0].label.contains("第 7 行"), "{}", all[0].label);
            assert!(!all.iter().any(|hit| hit.note == trashed));
            assert_eq!(db.task_overview(TaskScope::Board(DEFAULT_BOARD_ID), None, "")?.0.len(), 2);
            assert_eq!(db.task_overview(TaskScope::Note(first), None, "程式碼")?.0.len(), 0);
            assert_eq!(db.task_overview(TaskScope::Note(first), None, "隱藏屬性")?.0.len(), 0);
            let before = db.note(first)?.unwrap();
            let (_, span) = db.task_position(&all[0])?;
            let units = before.body.encode_utf16().collect::<Vec<_>>();
            assert_eq!(String::from_utf16(&units[span.start..span.end])?, "[ ]");
            db.toggle_listed_task(&all[0])?;
            assert!(db.toggle_listed_task(&all[0]).is_err());
            let changed = db.note(first)?.unwrap();
            assert_eq!(changed.body, before.body.replacen("- [ ] 確認分鏡", "- [x] 確認分鏡", 1));
            assert_eq!(db.note(second)?.unwrap().body.matches("確認分鏡").count(), 2);
            let on_board = db.task_overview(TaskScope::Board(DEFAULT_BOARD_ID), Some(true), "")?.0.remove(0);
            db.remove_card_on(DEFAULT_BOARD_ID, first)?;
            assert!(db.toggle_listed_task(&on_board).is_err());
        }
        let db = Notebook::open(&path, false)?;
        let (completed, _) = db.task_overview(TaskScope::Workspace, Some(true), "確認分鏡")?;
        assert_eq!(completed.iter().map(|hit| hit.note).collect::<Vec<_>>(), [first, second]);
        db.toggle_listed_task(&completed[0])?;
        assert_eq!(db.task_overview(TaskScope::Note(first), Some(true), "")?.0.len(), 0);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
