use super::{Notebook, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

#[path = "note_history_diff.rs"]
pub(crate) mod diff;

// Saving a draft and recording the previous revision are one SQLite operation.
// The live note is outside the bounded history, even when old revisions age out.
pub(super) const SCHEMA: &str = "
CREATE TABLE note_revisions(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    prior_version INTEGER NOT NULL,
    captured_at INTEGER NOT NULL DEFAULT (unixepoch()),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    snapshot_bytes INTEGER NOT NULL
);
CREATE INDEX note_revisions_by_note ON note_revisions(note_id,id DESC);
CREATE TRIGGER record_prior_note_revision AFTER UPDATE OF title,body ON notes
WHEN OLD.title <> NEW.title OR OLD.body <> NEW.body
BEGIN
    INSERT INTO note_revisions(note_id,prior_version,title,body,snapshot_bytes)
    VALUES(OLD.id,OLD.version,OLD.title,OLD.body,
           length(CAST(OLD.title AS BLOB))+length(CAST(OLD.body AS BLOB)));
    DELETE FROM note_revisions WHERE note_id=OLD.id AND id < (
        SELECT id FROM note_revisions WHERE note_id=OLD.id ORDER BY id DESC LIMIT 1 OFFSET 29
    );
    DELETE FROM note_revisions WHERE id < (
        SELECT id FROM note_revisions ORDER BY id DESC LIMIT 1 OFFSET 2047
    );
    DELETE FROM note_revisions WHERE id IN (
        SELECT id FROM (
            SELECT id,sum(snapshot_bytes) OVER (ORDER BY id DESC ROWS UNBOUNDED PRECEDING) AS total
            FROM note_revisions
        ) WHERE total>134217728
    );
END;
";

pub(crate) struct Revision {
    pub id: i64,
    pub version: i64,
    pub timestamp: String,
    pub title: String,
}

impl Notebook {
    pub(crate) fn note_revisions(&self, note: i64) -> Result<Vec<Revision>> {
        let mut statement = self.connection.prepare(
            "SELECT r.id,r.prior_version,strftime('%Y-%m-%d %H:%M:%SZ',r.captured_at,'unixepoch'),r.title
             FROM note_revisions r JOIN notes n ON n.id=r.note_id
             WHERE r.note_id=?1 AND n.trash_root IS NULL ORDER BY r.id DESC",
        )?;
        let rows = statement.query_map([note], |row| {
            Ok(Revision {
                id: row.get(0)?,
                version: row.get(1)?,
                timestamp: row.get(2)?,
                title: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn note_revision(&self, note: i64, revision: i64) -> Result<(String, String)> {
        revision_content(&self.connection, note, revision)
    }

    pub(crate) fn restore_note_revision(&self, note: i64, expected: i64, revision: i64) -> Result<bool> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let (title, body) = revision_content(&tx, note, revision)?;
        let current = tx.query_row(
            "SELECT title,body FROM notes WHERE id=?1 AND version=?2 AND trash_root IS NULL",
            params![note, expected], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        ).optional()?.ok_or("目前版本已變更。請重新開啟筆記並預覽。")?;
        if current.0 == title && current.1 == body {
            return Ok(false);
        }
        super::save_in(&tx, note, expected, &title, &body)?;
        tx.commit()?;
        Ok(true)
    }
}

fn revision_content(conn: &Connection, note: i64, revision: i64) -> Result<(String, String)> {
    conn.query_row(
        "SELECT r.title,r.body FROM note_revisions r JOIN notes n ON n.id=r.note_id
         WHERE r.note_id=?1 AND r.id=?2 AND n.trash_root IS NULL",
        params![note, revision], |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()?
    .ok_or_else(|| "這個舊版本已超過保留上限，或筆記在垃圾桶。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_restores_words_across_reopen_and_retains_the_replaced_version() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("history.db");
        let output = dir.join("backup.db");
        let (note, same_name) = {
            let mut db = Notebook::open(&path, false)?;
            let note = db.add(None, "舊標題", "第一版\r\n第二行")?;
            let same_name = db.add(None, "舊標題", "另一篇")?;
            db.set_property(note, "欄位", Some("保持"))?;
            let version = db.note(note)?.unwrap().version;
            db.save(note, version, "目前標題", "目前正文")?;
            assert!(db.note_revisions(same_name)?.is_empty());
            (note, same_name)
        };
        let db = Notebook::open(&path, false)?;
        let old = db.note_revisions(note)?.remove(0);
        assert_eq!(old.title, "舊標題");
        assert!(old.timestamp.ends_with('Z'));
        assert_eq!(db.note_revision(note, old.id)?.1, "第一版\r\n第二行");
        let version = db.note(note)?.unwrap().version;
        let competing = Notebook::open(&path, false)?;
        let newest = competing.save(note, version, "他人標題", "他人正文")?;
        assert!(db.restore_note_revision(note, version, old.id).is_err());
        assert!(db.restore_note_revision(same_name, 1, old.id).is_err());
        assert_eq!(db.note(note)?.unwrap().body, "他人正文");
        db.restore_note_revision(note, newest, old.id)?;
        db.backup(&output)?;
        drop(competing);
        drop(db);

        let restored = Notebook::open(&output, false)?;
        let saved = restored.note(note)?.unwrap();
        assert_eq!(saved.row.title, "舊標題");
        assert_eq!(saved.body, "第一版\r\n第二行");
        assert_eq!(restored.properties(note)?, [("欄位".to_owned(), "保持".to_owned())]);
        let mut revisions = restored.note_revisions(note)?;
        let prior = revisions.remove(0);
        assert_eq!((prior.version, prior.title.as_str()), (newest, "他人標題"));
        assert_eq!(restored.note_revision(note, prior.id)?.1, "他人正文");
        assert_eq!(revisions.len(), 2);
        drop(restored);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn history_bounds_per_note_and_tracks_non_editor_writes() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let mut db = Notebook::open(&dir.join("history.db"), false)?;
        let note = db.add(None, "長筆記", "0")?;
        let version = db.note(note)?.unwrap().version;
        db.save(note, version, "長筆記", "0")?;
        assert!(db.note_revisions(note)?.is_empty());
        for next in 1..=35 {
            db.connection.execute(
                "UPDATE notes SET body=?2,version=version+1 WHERE id=?1",
                params![note, next.to_string()],
            )?;
        }
        let revisions = db.note_revisions(note)?;
        assert_eq!(revisions.len(), 30);
        assert_eq!(db.note_revision(note, revisions[0].id)?.1, "34");
        assert_eq!(db.note_revision(note, revisions[29].id)?.1, "5");
        assert_eq!(db.note(note)?.unwrap().body, "35");
        db.trash(note)?;
        assert!(db.note_revisions(note)?.is_empty());
        db.restore(note)?;
        assert_eq!(db.note_revisions(note)?.len(), 30);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn equal_restore_is_a_noop_and_blank_restore_keeps_the_replaced_text() -> Result<()> {
        let mut db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let note = db.add(None, "草稿", "")?;
        let version = db.note(note)?.unwrap().version;
        let filled = db.save(note, version, "草稿", "甲\r\n\r\n乙")?;
        let cleared = db.save(note, filled, "草稿", "")?;
        let latest = db.save(note, cleared, "草稿", "甲\r\n\r\n乙")?;
        let revisions = db.note_revisions(note)?;
        assert_eq!(revisions.len(), 3);
        assert!(!db.restore_note_revision(note, latest, revisions[1].id)?);
        assert_eq!(db.note(note)?.unwrap().version, latest);
        assert_eq!(db.note_revisions(note)?.len(), 3);
        assert!(db.restore_note_revision(note, latest, revisions[0].id)?);
        assert_eq!(db.note(note)?.unwrap().body, "");
        let replaced = db.note_revisions(note)?.remove(0);
        assert_eq!(db.note_revision(note, replaced.id)?.1, "甲\r\n\r\n乙");
        Ok(())
    }
}
