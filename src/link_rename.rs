use super::super::Result;
use rusqlite::{Connection, params};
use std::collections::HashSet;

pub(crate) const RENAME_SCHEMA: &str = "
CREATE TABLE note_title_link_bindings(
    source_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    destination TEXT NOT NULL,
    target_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    PRIMARY KEY(source_id,destination)
) WITHOUT ROWID;
";

const MAX_RENAME_SCAN_BYTES: usize = 64 * 1024 * 1024;

/// Keep already-resolved bare WikiLinks on the original target ID. A historical
/// title never becomes an alias for new links elsewhere in the workspace.
/// Called in the same write transaction, after checking the expected version
/// and immediately before updating the note.
pub(crate) fn update_rename_bindings(
    conn: &Connection, id: i64, prior_title: &str, new_title: &str, body: &str,
) -> Result<()> {
    let remaining = keep_current_destinations(conn, id, body)?;
    if prior_title == new_title || prior_title.contains(['/', '\\', ':', '#']) {
        return Ok(());
    }
    let duplicate = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM notes WHERE title=?1 AND id<>?2 AND trash_root IS NULL)",
        params![prior_title, id], |row| row.get::<_, bool>(0),
    )?;
    if duplicate {
        return Ok(());
    }

    // The WikiLink parser returns the raw destination, so a source must contain
    // these exact title bytes. The parser still decides what is a real link.
    let mut query = conn.prepare(
        "SELECT n.id,n.body FROM notes n
         WHERE n.trash_root IS NULL AND instr(n.body,?1)>0 AND instr(n.body,'[[')>0
           AND NOT EXISTS(SELECT 1 FROM vault_paths p WHERE p.note_id=n.id)
           AND NOT EXISTS(SELECT 1 FROM note_title_link_bindings b
                          WHERE b.source_id=n.id AND b.destination=?1)",
    )?;
    let mut rows = query.query([prior_title])?;
    let mut sources = Vec::new();
    let mut consumed = 0usize;
    while let Some(row) = rows.next()? {
        let source: i64 = row.get(0)?;
        let previous = row.get_ref(1)?.as_str()?;
        consumed = consumed.saturating_add(previous.len());
        if consumed > MAX_RENAME_SCAN_BYTES {
            return Err("需檢查的連結正文超過 64 MiB，這次改名未儲存。編輯內容仍保留。".into());
        }
        if source == id && !remaining.as_ref().map_or_else(
            || crate::markdown::wiki_targets(body).iter().any(|key| key == prior_title),
            |destinations| destinations.contains(prior_title),
        ) {
            continue;
        }
        if crate::markdown::wiki_targets(previous).iter().any(|key| key == prior_title) {
            sources.push(source);
        }
    }
    let mut insert = conn.prepare(
        "INSERT INTO note_title_link_bindings(source_id,destination,target_id) VALUES(?1,?2,?3)",
    )?;
    for source in sources {
        insert.execute(params![source, prior_title, id])?;
    }
    Ok(())
}

fn keep_current_destinations(
    conn: &Connection, source: i64, body: &str,
) -> Result<Option<HashSet<String>>> {
    let candidates = {
        let mut stmt = conn.prepare_cached(
            "SELECT destination FROM note_title_link_bindings WHERE source_id=?1",
        )?;
        stmt.query_map([source], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if candidates.is_empty() {
        return Ok(None);
    }
    let current: HashSet<String> = crate::markdown::wiki_targets(body).into_iter().collect();
    let mut delete = conn.prepare_cached(
        "DELETE FROM note_title_link_bindings WHERE source_id=?1 AND destination=?2",
    )?;
    for old in candidates.iter().filter(|old| !current.contains(*old)) {
        delete.execute(params![source, old])?;
    }
    Ok(Some(current))
}

#[cfg(test)]
mod tests {
    use super::super::super::{Notebook, Note};
    use super::*;
    use std::path::Path;

    fn outgoing(db: &Notebook, source: i64) -> Result<Vec<i64>> {
        let note = db.note(source)?.unwrap();
        outgoing_from(db, &note)
    }

    fn outgoing_from(db: &Notebook, note: &Note) -> Result<Vec<i64>> {
        Ok(db.related(note)?.0.iter().map(|row| row.id).collect())
    }

    #[test]
    fn workspace_link_stays_with_renamed_note_across_reopen_title_reuse_and_restore() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let live = dir.join("live.sqlite3");
        let backup = dir.join("backup.sqlite3");
        let body = "開頭\r\n[[  舊名  |場記]] 和 `[[程式碼]]`";
        let (source, target, reuse) = {
            let mut db = Notebook::open(&live, false)?;
            let target = db.add(None, "舊名", "")?;
            let source = db.add(None, "來源", body)?;
            let literal_source = db.add(None, "程式碼", "`[[舊名]]`")?;
            assert_eq!(outgoing(&db, source)?, [target]);
            let renamed = db.note(target)?.unwrap();
            db.save(target, renamed.version, "第一次改名", "")?;
            db.save(target, renamed.version + 1, "第二次改名", "")?;
            assert!(db.save(target, renamed.version, "過期儲存", "").is_err());
            assert_eq!(db.note(target)?.unwrap().row.title, "第二次改名");
            let reuse = db.add(None, "舊名", "")?;
            let fresh = db.add(None, "新增來源", body)?;
            assert_eq!(outgoing(&db, source)?, [target]);
            assert_eq!(outgoing(&db, fresh)?, [reuse]);
            assert!(outgoing(&db, literal_source)?.is_empty());
            assert_eq!(db.note(source)?.unwrap().body, body);
            db.trash(target)?;
            assert!(outgoing(&db, source)?.is_empty());
            db.restore(target)?;
            db.backup(&backup)?;
            (source, target, reuse)
        };
        for path in [live, backup] {
            let db = Notebook::open(&path, false)?;
            let source_note = db.note(source)?.unwrap();
            assert_eq!(source_note.body, body);
            assert_eq!(outgoing_from(&db, &source_note)?, [target]);
            let (_, incoming) = db.graph_neighbors(&db.note(target)?.unwrap())?;
            assert_eq!(incoming.iter().map(|r| r.0).collect::<Vec<_>>(), [source]);
            assert_eq!(db.note(reuse)?.unwrap().row.title, "舊名");
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn save_prunes_removed_destinations_and_stale_views_use_committed_body() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let target = db.add(None, "重用", "")?;
        let source = db.add(None, "來源", "[[重用|段落]]")?;
        let stale = db.note(source)?.unwrap();
        db.save(target, 1, "已改名", "")?;
        let reuse = db.add(None, "重用", "")?;
        let version = db.save(source, stale.version, "來源", "追加\n[[重用|改別名]]")?;
        assert_eq!(outgoing(&db, source)?, [target]);
        let version = db.save(source, version, "來源", "`[[重用|改別名]]`")?;
        assert!(outgoing_from(&db, &stale)?.is_empty());
        db.save(source, version, "來源", "[[重用|重新選擇]]")?;
        assert_eq!(outgoing(&db, source)?, [reuse]);
        assert!(db.save(source, stale.version, "舊視窗", "").is_err());
        assert_eq!(outgoing(&db, source)?, [reuse]);
        Ok(())
    }

    #[test]
    fn ambiguous_titles_and_links_created_in_same_save_are_never_bound() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let target = db.add(None, "同名", "")?;
        let duplicate = db.add(None, "同名", "")?;
        let source = db.add(None, "來源", "[[同名]]")?;
        assert!(outgoing(&db, source)?.is_empty());
        db.save(target, 1, "改名", "")?;
        assert_eq!(outgoing(&db, source)?, [duplicate]);
        db.trash(duplicate)?;
        assert!(outgoing(&db, source)?.is_empty());
        let self_link = db.add(None, "新連結", "")?;
        db.save(self_link, 1, "新標題", "[[新連結]]")?;
        assert!(outgoing(&db, self_link)?.is_empty());
        Ok(())
    }
}
