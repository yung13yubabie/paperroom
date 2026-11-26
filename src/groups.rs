use super::{Notebook, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub(super) const SCHEMA: &str = "
CREATE TABLE board_groups(
    id INTEGER PRIMARY KEY,
    board_id INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    UNIQUE(board_id,id),
    UNIQUE(board_id,title)
);
CREATE TABLE board_group_cards(
    board_id INTEGER NOT NULL,
    note_id INTEGER NOT NULL,
    group_id INTEGER NOT NULL,
    PRIMARY KEY(board_id,note_id),
    FOREIGN KEY(board_id,note_id) REFERENCES board_cards(board_id,note_id) ON DELETE CASCADE,
    FOREIGN KEY(board_id,group_id) REFERENCES board_groups(board_id,id) ON DELETE CASCADE
);
";

pub(super) const SOURCE_SCHEMA: &str = "
CREATE TABLE board_group_sources(
    board_id INTEGER NOT NULL,
    group_id INTEGER NOT NULL,
    annotation_id INTEGER NOT NULL REFERENCES pdf_annotations(id) ON DELETE CASCADE,
    PRIMARY KEY(board_id,group_id),
    FOREIGN KEY(board_id,group_id) REFERENCES board_groups(board_id,id) ON DELETE CASCADE
);
CREATE INDEX board_group_sources_annotation ON board_group_sources(annotation_id);
";

#[cfg(any(windows, test))]
const LIVE_GROUP_ANNOTATION: &str = "SELECT p.id
    FROM pdf_annotations p
    JOIN attachments a ON a.id=p.attachment_id AND a.kind='pdf'
    JOIN notes n ON n.id=a.note_id AND n.trash_root IS NULL
    JOIN board_group_cards m ON m.note_id=n.id
    WHERE m.board_id=?1 AND m.group_id=?2 AND p.id=?3";

#[cfg(any(windows, test))]
fn source_id(connection: &Connection, board: i64, group: i64) -> Result<Option<i64>> {
    connection.query_row(
        "SELECT s.annotation_id FROM board_groups g LEFT JOIN board_group_sources s
         ON s.board_id=g.board_id AND s.group_id=g.id WHERE g.board_id=?1 AND g.id=?2",
        params![board, group],
        |row| row.get(0),
    ).optional()?.ok_or_else(|| "這個分區不在目前白板，或已被移除。".into())
}

pub struct BoardGroup {
    pub id: i64,
    pub title: String,
    pub members: Vec<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupCardPosition {
    pub note_id: i64,
    pub x: i32,
    pub y: i32,
}

fn checked_title(title: &str) -> Result<&str> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 80 || title.chars().any(char::is_control) {
        return Err("分區名稱須為 1 至 80 個字，且不能包含控制字元。".into());
    }
    Ok(title)
}

fn active_positions(connection: &Connection, board: i64, group: i64) -> Result<Vec<GroupCardPosition>> {
    let mut statement = connection.prepare(
        "SELECT g.id,c.note_id,c.x,c.y FROM board_groups g
         LEFT JOIN (
             SELECT m.board_id,m.group_id,b.note_id,b.x,b.y FROM board_group_cards m
             JOIN board_cards b ON b.board_id=m.board_id AND b.note_id=m.note_id
             JOIN notes n ON n.id=m.note_id AND n.trash_root IS NULL
         ) c ON c.board_id=g.board_id AND c.group_id=g.id
         WHERE g.board_id=?1 AND g.id=?2 ORDER BY c.note_id",
    )?;
    let mut rows = statement.query(params![board, group])?;
    let Some(first) = rows.next()? else {
        return Err("這個分區不在目前白板，或已被移除。".into());
    };
    if first.get::<_, Option<i64>>(1)?.is_none() {
        return Ok(Vec::new());
    }
    let position = |row: &rusqlite::Row<'_>| -> rusqlite::Result<GroupCardPosition> {
        Ok(GroupCardPosition { note_id: row.get(1)?, x: row.get(2)?, y: row.get(3)? })
    };
    let mut result = vec![position(first)?];
    while let Some(row) = rows.next()? {
        result.push(position(row)?);
    }
    Ok(result)
}

impl Notebook {
    #[cfg(any(windows, test))]
    pub fn board_group_source_id(&self, board: i64, group: i64) -> Result<Option<i64>> {
        source_id(&self.connection, board, group)
    }

    #[cfg(any(windows, test))]
    pub fn live_board_group_source(
        &self, board: i64, group: i64, annotation: i64,
    ) -> Result<Option<super::attachments::PdfAnnotationHit>> {
        let mut query = String::from(LIVE_GROUP_ANNOTATION);
        query.push_str(" AND EXISTS (SELECT 1 FROM board_group_sources
            WHERE board_id=?1 AND group_id=?2 AND annotation_id=?3)");
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        let live = tx.query_row(&query, params![board, group, annotation], |row| {
            row.get::<_, i64>(0)
        }).optional()?;
        let source = match live {
            Some(id) => self.pdf_annotation_hit(id)?,
            None => None,
        };
        tx.commit()?;
        Ok(source)
    }

    #[cfg(any(windows, test))]
    pub fn set_board_group_source(
        &self, board: i64, group: i64, expected: Option<i64>, next: Option<i64>,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if source_id(&tx, board, group)? != expected {
            return Err("分區釘選已變更，請重新開啟選單。".into());
        }
        if let Some(annotation) = next {
            let live = tx.query_row(LIVE_GROUP_ANNOTATION, params![board, group, annotation], |row| {
                row.get::<_, i64>(0)
            }).optional()?;
            if live.is_none() {
                return Err("標註已移除，或所屬筆記已離開這個分區。".into());
            }
            tx.execute(
                "INSERT INTO board_group_sources(board_id,group_id,annotation_id) VALUES(?1,?2,?3)
                 ON CONFLICT(board_id,group_id) DO UPDATE SET annotation_id=excluded.annotation_id",
                params![board, group, annotation],
            )?;
        } else {
            tx.execute("DELETE FROM board_group_sources WHERE board_id=?1 AND group_id=?2",
                params![board, group])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn create_board_group(&self, board: i64, title: &str) -> Result<i64> {
        let title = checked_title(title)?;
        let changed = self.connection.execute(
            "INSERT INTO board_groups(board_id,title)
             SELECT ?1,?2 FROM boards WHERE id=?1
             AND (SELECT count(*) FROM board_groups WHERE board_id=?1)<64
             ON CONFLICT(board_id,title) DO NOTHING",
            params![board, title],
        )?;
        if changed != 1 {
            return Err("白板不存在、分區名稱已使用，或已達 64 個分區。".into());
        }
        Ok(self.connection.last_insert_rowid())
    }

    pub fn rename_board_group(
        &self, board: i64, group: i64, expected_title: &str, new_title: &str,
    ) -> Result<()> {
        let title = checked_title(new_title)?;
        let changed = self.connection.execute(
            "UPDATE board_groups SET title=?4 WHERE board_id=?1 AND id=?2 AND title=?3
             AND NOT EXISTS(SELECT 1 FROM board_groups WHERE board_id=?1 AND id<>?2 AND title=?4)",
            params![board, group, expected_title, title],
        )?;
        if changed != 1 {
            return Err("分區已改變、已移除，或此白板已有同名分區。".into());
        }
        Ok(())
    }

    pub fn board_group_positions(&self, board: i64, group: i64) -> Result<Vec<GroupCardPosition>> {
        active_positions(&self.connection, board, group)
    }

    /// Move exactly the active placements returned by board_group_positions.
    pub fn move_board_group(
        &self, board: i64, group: i64, expected: &[GroupCardPosition], dx: i32, dy: i32,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let current = active_positions(&tx, board, group)?;
        if current != expected {
            return Err("分區成員或位置已改變，請重新開啟分區後再移動。".into());
        }
        if current.is_empty() {
            return Err("這個分區沒有可移動的卡片。".into());
        }
        let mut moved = Vec::with_capacity(current.len());
        for entry in &current {
            let x = entry.x.checked_add(dx).filter(|x| (0..=10000).contains(x));
            let y = entry.y.checked_add(dy).filter(|y| (0..=50000).contains(y));
            let (Some(x), Some(y)) = (x, y) else {
                return Err("整組位移會超出白板範圍，未移動任何卡片。".into());
            };
            moved.push((entry.note_id, x, y));
        }
        {
            let mut write = tx.prepare("UPDATE board_cards SET x=?3,y=?4 WHERE board_id=?1 AND note_id=?2")?;
            for (id,x,y) in moved {
                if write.execute(params![board,id,x,y])? != 1 {
                    return Err("卡片已變更，未移動任何卡片。".into());
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn groups_on(&self, board: i64) -> Result<Vec<BoardGroup>> {
        let mut stmt = self.connection.prepare(
            "SELECT g.id,g.title,n.id FROM board_groups g
             LEFT JOIN board_group_cards c ON c.board_id=g.board_id AND c.group_id=g.id
             LEFT JOIN notes n ON n.id=c.note_id AND n.trash_root IS NULL
             WHERE g.board_id=?1 ORDER BY g.id, n.id",
        )?;
        let mut query = stmt.query([board])?;
        let mut groups: Vec<BoardGroup> = Vec::new();
        while let Some(row) = query.next()? {
            let id: i64 = row.get(0)?;
            if groups.last().is_none_or(|group| group.id != id) {
                groups.push(BoardGroup { id, title: row.get(1)?, members: Vec::new() });
            }
            if let Some(note) = row.get::<_, Option<i64>>(2)? {
                groups.last_mut().expect("current group was loaded").members.push(note);
            }
        }
        Ok(groups)
    }

    pub fn set_board_group(&self, board: i64, note: i64, expected: Option<i64>, group: Option<i64>) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let active = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM board_cards c JOIN notes n ON n.id=c.note_id
                           WHERE c.board_id=?1 AND c.note_id=?2 AND n.trash_root IS NULL)",
            params![board, note], |row| row.get::<_, bool>(0),
        )?;
        let current = tx.query_row(
            "SELECT group_id FROM board_group_cards WHERE board_id=?1 AND note_id=?2",
            params![board, note], |row| row.get::<_, i64>(0),
        ).optional()?;
        if !active || current != expected {
            return Err("這張卡片的白板或分區已改變，請重新開啟白板。".into());
        }
        if let Some(group) = group {
            let changed = tx.execute(
                "INSERT INTO board_group_cards(board_id,note_id,group_id)
                 SELECT ?1,?2,id FROM board_groups WHERE board_id=?1 AND id=?3
                 ON CONFLICT(board_id,note_id) DO UPDATE SET group_id=excluded.group_id",
                params![board, note, group],
            )?;
            if changed != 1 {
                return Err("這個分區不在目前白板。".into());
            }
        } else {
            tx.execute("DELETE FROM board_group_cards WHERE board_id=?1 AND note_id=?2", params![board, note])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::DEFAULT_BOARD_ID;
    use std::path::Path;

    #[test]
    fn section_rename_and_translation_preserve_legacy_board_data() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("schema-six.sqlite3");
        let backup = dir.join("backup.sqlite3");
        {
            let legacy = Connection::open(&path)?;
            legacy.pragma_update(None, "foreign_keys", true)?;
            legacy.execute_batch(
                "CREATE TABLE notes(id INTEGER PRIMARY KEY,parent INTEGER,title TEXT NOT NULL,body TEXT NOT NULL,
                    pinned INTEGER NOT NULL,trash_root INTEGER,version INTEGER NOT NULL);
                 CREATE TABLE vault_paths(note_id INTEGER PRIMARY KEY REFERENCES notes(id),
                    import_root INTEGER NOT NULL REFERENCES notes(id), relative_path TEXT NOT NULL,
                    UNIQUE(import_root,relative_path));
                 CREATE TABLE boards(id INTEGER PRIMARY KEY,title TEXT NOT NULL);
                 CREATE TABLE board_cards(board_id INTEGER NOT NULL REFERENCES boards(id),
                    note_id INTEGER NOT NULL REFERENCES notes(id),x INTEGER NOT NULL,y INTEGER NOT NULL,
                    PRIMARY KEY(board_id,note_id));
                 INSERT INTO boards VALUES(1,'白板'),(2,'另一白板');
                 INSERT INTO notes VALUES
                    (10,NULL,'第一張','',0,NULL,1),
                    (11,NULL,'第二張','',0,NULL,1),
                    (12,NULL,'垃圾桶內','',0,12,1),
                    (13,NULL,'不在分區','',0,NULL,1);
                 INSERT INTO board_cards VALUES
                    (1,10,25,300),(1,11,60,360),(1,12,95,420),(1,13,3,7),(2,10,14,22);",
            )?;
            legacy.execute_batch(SCHEMA)?;
            legacy.execute_batch(super::super::attachments::SCHEMA)?;
            legacy.execute_batch(super::super::pdf_regions::SCHEMA)?;
            legacy.execute_batch(super::super::collections::SCHEMA)?;
            legacy.execute_batch(super::super::collection_views::SCHEMA)?;
            legacy.execute_batch(super::super::workspace::SCHEMA)?;
            legacy.execute_batch(
                "INSERT INTO board_groups VALUES(2,1,'舊分區'),(3,1,'占用'),(4,2,'舊分區');
                 INSERT INTO board_group_cards VALUES(1,10,2),(1,11,2),(1,12,2),(2,10,4);
                 PRAGMA user_version=6;",
            )?;
        }
        {
            let db = Notebook::open(&path, false)?;
            let opened_elsewhere = Notebook::open(&path, false)?;
            assert!(db.rename_board_group(1, 2, "舊分區", "占用").is_err());
            assert!(db.rename_board_group(2, 2, "舊分區", "錯誤白板").is_err());
            db.rename_board_group(1, 2, "舊分區", " 已重新命名 ")?;
            assert!(opened_elsewhere.rename_board_group(1, 2, "舊分區", "舊快照").is_err());
            assert_eq!(db.groups_on(2)?[0].title, "舊分區");

            let initial = db.board_group_positions(1, 2)?;
            assert_eq!(initial.iter().map(|card| card.note_id).collect::<Vec<_>>(), [10,11]);
            assert!(db.move_board_group(2, 2, &initial, 5, 5).is_err());
            assert!(db.move_board_group(1, 2, &initial, -50, 0).is_err());
            assert!(db.move_board_group(1, 2, &initial, i32::MAX, 0).is_err());
            assert_eq!(db.board_group_positions(1, 2)?, initial);

            opened_elsewhere.move_card_on(1, 11, 61, 361)?;
            assert!(db.move_board_group(1, 2, &initial, 20, -10).is_err());
            let snapshot = db.board_group_positions(1, 2)?;
            db.connection.execute_batch(
                "CREATE TEMP TRIGGER reject_second_card BEFORE UPDATE ON board_cards
                 WHEN old.note_id=11 BEGIN SELECT RAISE(ABORT, 'test rollback'); END;",
            )?;
            assert!(db.move_board_group(1, 2, &snapshot, 20, -10).is_err());
            assert_eq!(opened_elsewhere.board_group_positions(1, 2)?, snapshot);
            db.connection.execute_batch("DROP TRIGGER reject_second_card")?;
            db.move_board_group(1, 2, &snapshot, 20, -10)?;
            let moved = db.board_group_positions(1, 2)?;
            assert_eq!(moved.iter().map(|c| (c.note_id,c.x,c.y)).collect::<Vec<_>>(),
                       [(10,45,290),(11,81,351)]);
            assert_eq!(db.board_group_positions(2, 4)?, [GroupCardPosition { note_id: 10, x:14, y:22 }]);
            let unchanged = |note: i64| -> Result<(i32,i32)> {
                Ok(db.connection.query_row("SELECT x,y FROM board_cards WHERE board_id=1 AND note_id=?1",
                    [note], |row| Ok((row.get(0)?,row.get(1)?)))?)
            };
            assert_eq!(unchanged(12)?, (95,420));
            assert_eq!(unchanged(13)?, (3,7));
            db.backup(&backup)?;
            let restored = Notebook::open(&backup,false)?;
            assert_eq!(restored.groups_on(1)?[0].title, "已重新命名");
            assert_eq!(restored.board_group_positions(1,2)?, moved);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn legacy_group_pins_exact_pdf_annotation_and_rejects_stale_changes() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let backup = dir.join("backup.sqlite3");
        let source = dir.join("同名來源.pdf");
        std::fs::write(&source, b"%PDF-2.0\n")?;
        let region = crate::store::PdfRegion { x: 0.1, y: 0.2, width: 0.5, height: 0.6 };
        let board = DEFAULT_BOARD_ID;
        let (group, another, member, file, first, next, outsider) = {
            let mut db = Notebook::open(&path, false)?;
            let group = db.create_board_group(board, "來源")?;
            let another = db.create_board("另一塊白板")?;
            let member = db.add(None, "同名筆記", "")?;
            let outside = db.add(None, "同名筆記", "")?;
            db.add_card_on(board, member)?;
            db.add_card_on(board, outside)?;
            db.set_board_group(board, member, None, Some(group))?;
            let file = db.import_attachment(member, &source)?;
            let other_file = db.import_attachment(outside, &source)?;
            let first = db.add_pdf_annotation(file, 1, "較舊引文", "")?;
            let next = db.add_pdf_annotation_with_region(file, 2, "分區來源", "", region)?;
            let outsider = db.add_pdf_annotation(other_file, 2, "分區來源", "")?;
            crate::store::tests::remove_v11_schema(&db.connection)?;
            db.connection.execute_batch("DROP TABLE pdf_text_mark_segments; DROP TABLE vault_push_verified; DROP TABLE pdf_text_marks; DROP TABLE vault_sync_base; DROP TABLE attachment_origins; DROP TABLE board_group_sources; PRAGMA user_version=7;")?;
            (group, another, member, file, first, next, outsider)
        };
        {
            let mut db = Notebook::open(&path, false)?;
            let other_session = Notebook::open(&path, false)?;
            assert_eq!(db.board_group_source_id(board, group)?, None);
            assert!(db.set_board_group_source(another, group, None, Some(next)).is_err());
            assert!(db.set_board_group_source(board, group, None, Some(outsider)).is_err());
            db.set_board_group_source(board, group, None, Some(first))?;
            assert!(other_session.set_board_group_source(board, group, None, Some(next)).is_err());
            assert!(db.set_board_group_source(board, group, Some(first), Some(outsider)).is_err());
            assert_eq!(db.board_group_source_id(board, group)?, Some(first));
            db.set_board_group_source(board, group, Some(first), Some(next))?;
            assert!(db.live_board_group_source(board, group, first)?.is_none());
            db.trash(member)?;
            assert_eq!(db.board_group_source_id(board, group)?, Some(next));
            assert!(db.live_board_group_source(board, group, next)?.is_none());
            assert!(db.set_board_group_source(board, group, Some(next), Some(first)).is_err());
            db.restore(member)?;
            db.set_board_group(board, member, Some(group), None)?;
            assert!(db.live_board_group_source(board, group, next)?.is_none());
            db.set_board_group(board, member, None, Some(group))?;
            db.backup(&backup)?;
        }
        {
            let restored = Notebook::open(&backup, false)?;
            let hit = restored.live_board_group_source(board, group, next)?.ok_or("missing pin")?;
            assert_eq!((hit.note_id, hit.attachment_id, hit.id, hit.page), (member, file, next, 2));
            assert_eq!(restored.pdf_region(hit.id)?, Some(region));
        }
        {
            let reopened = Notebook::open(&path, false)?;
            assert_eq!(reopened.board_group_source_id(board, group)?, Some(next));
            reopened.remove_pdf_annotation(next)?;
            assert_eq!(reopened.board_group_source_id(board, group)?, None);
            assert!(reopened.live_board_group_source(board, group, next)?.is_none());
            reopened.set_board_group_source(board, group, None, Some(first))?;
            reopened.set_board_group_source(board, group, Some(first), None)?;
            assert_eq!(reopened.board_group_source_id(board, group)?, None);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn grouping_is_board_scoped_and_survives_trash() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let board = DEFAULT_BOARD_ID;
        let another = db.create_board("其他白板")?;
        let group = db.create_board_group(board, "資料")?;
        let outside = db.create_board_group(another, "不同分區")?;
        let note = db.add(None, "筆記", "")?;
        db.add_card_on(board, note)?;
        assert!(db.set_board_group(board, note, None, Some(outside)).is_err());
        db.set_board_group(board, note, None, Some(group))?;
        assert!(db.set_board_group(board, note, None, None).is_err());
        db.trash(note)?;
        assert!(db.groups_on(board)?[0].members.is_empty());
        db.restore(note)?;
        assert_eq!(db.groups_on(board)?[0].members, [note]);
        db.remove_card_on(board, note)?;
        db.add_card_on(board, note)?;
        assert!(db.groups_on(board)?[0].members.is_empty());
        Ok(())
    }
}
