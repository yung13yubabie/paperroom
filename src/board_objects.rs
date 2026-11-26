//! A child board has at most one incoming tile. Free text belongs to its board.
use super::boards::Board;
use super::{Notebook,Result,MAX_TITLE_CHARS,DEFAULT_BOARD_ID};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::HashSet;

pub(crate) const SCHEMA:&str="
CREATE TABLE board_objects(
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 board_id INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
 child_id INTEGER UNIQUE REFERENCES boards(id) ON DELETE CASCADE,
 body TEXT NOT NULL DEFAULT '',
 x INTEGER NOT NULL CHECK(x BETWEEN 0 AND 10000),
 y INTEGER NOT NULL CHECK(y BETWEEN 0 AND 50000),
 version INTEGER NOT NULL DEFAULT 1,
 CHECK(child_id IS NULL OR (child_id<>board_id AND child_id<>1 AND body=''))
);
CREATE INDEX board_objects_parent ON board_objects(board_id,id);";
pub(crate) const TEXT_RECOVERY_SCHEMA: &str = "
CREATE TABLE board_text_recovery(
 board_id INTEGER PRIMARY KEY REFERENCES boards(id) ON DELETE CASCADE,
 object_id INTEGER NOT NULL UNIQUE,
 body TEXT NOT NULL,
 x INTEGER NOT NULL CHECK(x BETWEEN 0 AND 10000),
 y INTEGER NOT NULL CHECK(y BETWEEN 0 AND 50000),
 version INTEGER NOT NULL
);";
const MAX_DEPTH:usize=16;
const LIMIT:i64=128;
pub(crate) const TEXT_LIMIT:usize=4096;

#[derive(Clone)]
pub(crate) struct Object {
    pub id:i64, pub child:Option<i64>, pub text:String,
    pub x:i32, pub y:i32, pub version:i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BoardItem {
    Card(i64),
    Object(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoardPosition {
    pub item: BoardItem,
    pub x: i32,
    pub y: i32,
    pub version: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RemovedText {
    id: i64,
    pub text: String,
    x: i32,
    y: i32,
    version: i64,
}

fn latest_removed(db: &Connection, board: i64) -> Result<Option<RemovedText>> {
    Ok(db.query_row(
        "SELECT object_id,body,x,y,version FROM board_text_recovery WHERE board_id=?1",
        [board],
        |row| Ok(RemovedText {
            id: row.get(0)?,
            text: row.get(1)?,
            x: row.get(2)?,
            y: row.get(3)?,
            version: row.get(4)?,
        }),
    ).optional()?)
}

fn parent(db:&Connection,board:i64)->Result<Option<i64>> {
    Ok(db.query_row("SELECT board_id FROM board_objects WHERE child_id=?1",[board],|r|r.get(0)).optional()?)
}

fn board_depth(db:&Connection,mut board:i64,moving:Option<i64>)->Result<usize>{
    let mut depth=0;
    loop {
        if Some(board)==moving {return Err("不能將白板放入自己或自己的子白板。".into());}
        let Some(next)=parent(db,board)? else{return Ok(depth)};
        depth+=1;
        if depth>MAX_DEPTH{return Err("白板階層超過 16 層。".into());}
        board=next;
    }
}

pub(super) fn position(db:&Connection,board:i64,preferred:Option<(i32,i32)>)->Result<(i32,i32)> {
    let count:i64=db.query_row("SELECT COUNT(*) FROM board_objects WHERE board_id=?1",[board],|r|r.get(0))?;
    if count>=LIMIT{return Err("每張白板最多可放 128 個子白板、文字方塊與心智圖節點。".into());}
    // Pick an unoccupied grid rectangle, including hand-positioned note cards.
    let (preferred_x, preferred_y) = preferred.unzip();
    let slot=db.query_row("WITH RECURSIVE grid(n,x,y) AS (
      VALUES(0,16,16) UNION ALL SELECT n+1,16+((n+1)%2)*260,16+((n+1)/2)*135 FROM grid WHERE n<737
    ), slots(n,x,y) AS (
      SELECT -1,?2,?3 WHERE ?2 IS NOT NULL
      UNION ALL SELECT n,x,y FROM grid
    ) SELECT x,y FROM slots s WHERE NOT EXISTS(
      SELECT 1 FROM board_card_geometry c WHERE c.board_id=?1 AND c.x<s.x+232 AND c.x+c.width>s.x
        AND c.y<s.y+114 AND c.y+c.height>s.y)
    AND NOT EXISTS(SELECT 1 FROM board_objects o WHERE o.board_id=?1 AND abs(o.x-s.x)<232 AND abs(o.y-s.y)<114)
    ORDER BY n LIMIT 1",params![board,preferred_x,preferred_y],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    slot.ok_or_else(||"白板格位已滿，請先調整物件位置。".into())
}

fn create_child_board(db: &Connection, parent_board: i64, title: &str) -> Result<i64> {
    if board_depth(db, parent_board, None)? >= MAX_DEPTH {
        return Err("白板階層最多 16 層。".into());
    }
    let (x, y) = position(db, parent_board, None)?;
    let child = super::boards::create_board_on(db, title)?;
    db.execute(
        "INSERT INTO board_objects(board_id,child_id,x,y) VALUES(?1,?2,?3,?4)",
        params![parent_board, child, x, y],
    )?;
    Ok(child)
}

#[path="board_selection_child.rs"]
mod selection_child;

fn valid_text(text:&str)->Result<()> {
    if text.trim().is_empty() || text.contains('\0') || text.chars().count()>TEXT_LIMIT {
        return Err("文字方塊須有內容，最多 4,096 字。".into());
    }
    Ok(())
}

impl Notebook {
    /// Translate the selected placements together, keeping IDs, order and links.
    /// Trashed notes still reserve positions because their placements can return.
    pub(crate) fn move_board_selection(
        &self, board: i64, expected: &[BoardPosition], dx: i32, dy: i32,
    ) -> Result<()> {
        let selected: HashSet<_> = expected.iter().map(|entry| entry.item).collect();
        if expected.is_empty() || selected.len() != expected.len() {
            return Err("請選取白板物件後重試。".into());
        }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let layout = {
            let mut query = tx.prepare(
                "SELECT 0,c.note_id,c.x,c.y,NULL,n.trash_root IS NULL,c.width,c.height FROM board_card_geometry c
                 JOIN notes n ON n.id=c.note_id WHERE c.board_id=?1
                 UNION ALL SELECT 1,id,x,y,version,1,232,114 FROM board_objects WHERE board_id=?1",
            )?;
            query.query_map([board], |row| {
                let id = row.get(1)?;
                let item = if row.get::<_, bool>(0)? { BoardItem::Object(id) } else { BoardItem::Card(id) };
                Ok((BoardPosition { item, x: row.get(2)?, y: row.get(3)?, version: row.get(4)? },
                    row.get::<_, bool>(5)?, row.get::<_, i32>(6)?, row.get::<_, i32>(7)?))
            })?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut positions = Vec::with_capacity(expected.len());
        for entry in expected {
            let Some((_, _, width, height)) = layout.iter().find(|(current, active, _, _)| *active && current == entry) else {
                return Err("白板物件或位置已變更，未移動任何物件。請重新選取。".into());
            };
            let x = entry.x.checked_add(dx).filter(|x| (0..=10000).contains(x));
            let y = entry.y.checked_add(dy).filter(|y| (0..=50000).contains(y));
            let (Some(x), Some(y)) = (x, y) else {
                return Err("整批位移會超出白板範圍，未移動任何物件。".into());
            };
            if (dx, dy) != (0, 0) && layout.iter().any(|(other, _, other_width, other_height)| {
                !selected.contains(&other.item) && x < other.x + other_width && x + width > other.x
                    && y < other.y + other_height && y + height > other.y
            }) {
                return Err("目的地與未選取的物件重疊，未移動任何物件。".into());
            }
            positions.push((entry.item, x, y));
        }
        if (dx, dy) != (0, 0) {
            let mut cards = tx.prepare("UPDATE board_cards SET x=?3,y=?4 WHERE board_id=?1 AND note_id=?2")?;
            let mut objects = tx.prepare("UPDATE board_objects SET x=?3,y=?4,version=version+1 WHERE board_id=?1 AND id=?2")?;
            for (item, x, y) in positions {
                let changed = match item {
                    BoardItem::Card(id) => cards.execute(params![board, id, x, y])?,
                    BoardItem::Object(id) => objects.execute(params![board, id, x, y])?,
                };
                if changed != 1 {
                    return Err("白板物件已變更，未移動任何物件。".into());
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn objects_on(&self,board:i64)->Result<Vec<Object>> {
        let mut stmt=self.connection.prepare("SELECT o.id,o.child_id,COALESCE(b.title,o.body),o.x,o.y,o.version
            FROM board_objects o LEFT JOIN boards b ON b.id=o.child_id WHERE o.board_id=?1 ORDER BY o.id")?;
        let items=stmt.query_map([board],|r|Ok(Object{id:r.get(0)?,child:r.get(1)?,text:r.get(2)?,x:r.get(3)?,y:r.get(4)?,version:r.get(5)?}))?;
        Ok(items.collect::<rusqlite::Result<_>>()?)
    }
    pub(crate) fn parent_board(&self,board:i64)->Result<Option<i64>> {parent(&self.connection,board)}
    pub(crate) fn board_path(&self, board: i64) -> Result<Vec<Board>> {
        let mut stmt = self.connection.prepare(
            "WITH RECURSIVE path(id,depth) AS (
                SELECT id,0 FROM boards WHERE id=?1
                UNION ALL
                SELECT o.board_id,path.depth+1 FROM path JOIN board_objects o ON o.child_id=path.id
                WHERE path.depth<?2
            ) SELECT b.id,b.title FROM path JOIN boards b ON b.id=path.id ORDER BY path.depth DESC",
        )?;
        let path = stmt.query_map(params![board, MAX_DEPTH as i64 + 1], |row| {
            Ok(Board { id: row.get(0)?, title: row.get(1)? })
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        if path.is_empty() {
            return Err("找不到白板。".into());
        }
        if path.len() > MAX_DEPTH + 1 {
            return Err("白板階層超過 16 層或出現循環。".into());
        }
        Ok(path)
    }
    pub(crate) fn new_child_board(&mut self, board: i64, title: &str) -> Result<i64> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let child = create_child_board(&tx, board, title)?;
        tx.commit()?;
        Ok(child)
    }

    pub(crate) fn new_child_board_from_card(
        &mut self, board: i64, note: i64, expected: (i32, i32), title: &str,
    ) -> Result<i64> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM board_cards c JOIN notes n ON n.id=c.note_id
             WHERE c.board_id=?1 AND c.note_id=?2 AND c.x=?3 AND c.y=?4 AND n.trash_root IS NULL)",
            params![board, note, expected.0, expected.1], |row| row.get(0),
        )?;
        if !active {
            return Err("來源卡片或位置已變更。請重新開啟白板並選取卡片。".into());
        }
        let child = create_child_board(&tx, board, title)?;
        super::boards::place_card_on(&tx, child, note)?;
        tx.commit()?;
        Ok(child)
    }

    /// Find the current parent by the child relationship, even after reopening.
    pub(crate) fn parent_board_tile(&self, child: i64) -> Result<Option<(Board, i64)>> {
        Ok(self.connection.query_row(
            "SELECT p.id,p.title,o.id FROM board_objects o JOIN boards p ON p.id=o.board_id
             WHERE o.child_id=?1", [child], |row| {
                Ok((Board { id: row.get(0)?, title: row.get(1)? }, row.get(2)?))
            },
        ).optional()?)
    }

    pub(crate) fn child_board_from_tile(&self, parent: i64, tile: i64, child: i64) -> Result<Board> {
        self.connection.query_row(
            "SELECT b.id,b.title FROM board_objects o JOIN boards b ON b.id=o.child_id
             WHERE o.board_id=?1 AND o.id=?2 AND o.child_id=?3",
            params![parent, tile, child],
            |row| Ok(Board { id: row.get(0)?, title: row.get(1)? }),
        ).optional()?.ok_or_else(|| "子白板已移動，請重新開啟白板。".into())
    }

    pub(crate) fn reparent_board(&mut self,board:i64,expected:Option<i64>,to:Option<i64>)->Result<()> {
        if board==DEFAULT_BOARD_ID{return Err("預設白板必須位於最上層。".into());}
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual=parent(&tx,board)?;
        if actual!=expected{return Err("上層白板已變更，請重新開啟選單。".into());}
        if actual==to{return Ok(());}
        if let Some(target)=to {
            let depth=board_depth(&tx,target,Some(board))?;
            let height:i64=tx.query_row("WITH RECURSIVE tree(id,d) AS (VALUES(?1,0) UNION ALL
              SELECT o.child_id,t.d+1 FROM tree t JOIN board_objects o ON o.board_id=t.id
              WHERE o.child_id IS NOT NULL AND t.d<17) SELECT MAX(d) FROM tree",[board],|r|r.get(0))?;
            if i64::try_from(depth)?+1+height>MAX_DEPTH as i64{return Err("移動後的白板階層超過 16 層。".into());}
            let (x,y)=position(&tx,target,None)?;
            tx.execute("INSERT INTO board_objects(board_id,child_id,x,y) VALUES(?1,?2,?3,?4)
                ON CONFLICT(child_id) DO UPDATE SET board_id=excluded.board_id,x=excluded.x,y=excluded.y,version=version+1",params![target,board,x,y])?;
        }else{tx.execute("DELETE FROM board_objects WHERE child_id=?1",[board])?;}
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn add_board_text(&mut self,board:i64,text:&str)->Result<i64>{
        valid_text(text)?;
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (x,y)=position(&tx,board,None)?;
        tx.execute("INSERT INTO board_objects(board_id,body,x,y) VALUES(?1,?2,?3,?4)",params![board,text,x,y])?;
        let id=tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }
    pub(crate) fn edit_board_text(&self,board:i64,o:&Object,text:&str)->Result<()> {
        valid_text(text)?;
        if self.connection.execute("UPDATE board_objects SET body=?1,version=version+1 WHERE board_id=?2 AND id=?3 AND version=?4 AND child_id IS NULL
            AND NOT EXISTS(SELECT 1 FROM board_mind_nodes WHERE id=board_objects.id)",
            params![text,board,o.id,o.version])?!=1{return Err("文字方塊已變更，請重新開啟。".into());}
        Ok(())
    }
    pub(crate) fn latest_removed_board_text(&self, board: i64) -> Result<Option<RemovedText>> {
        latest_removed(&self.connection, board)
    }

    pub(crate) fn remove_board_text(
        &mut self, board: i64, object: &Object, previous: Option<&RemovedText>,
    ) -> Result<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if latest_removed(&tx, board)?.as_ref() != previous {
            return Err("最近移除的文字方塊已變更，請重新確認。".into());
        }
        let saved = tx.execute(
            "INSERT INTO board_text_recovery(board_id,object_id,body,x,y,version)
             SELECT board_id,id,body,x,y,version FROM board_objects
             WHERE board_id=?1 AND id=?2 AND version=?3 AND child_id IS NULL
                AND NOT EXISTS(SELECT 1 FROM board_mind_nodes WHERE id=board_objects.id)
             ON CONFLICT(board_id) DO UPDATE SET object_id=excluded.object_id,body=excluded.body,
             x=excluded.x,y=excluded.y,version=excluded.version",
            params![board, object.id, object.version],
        )?;
        if saved != 1 {
            return Err("文字方塊已變更，請重新開啟白板。".into());
        }
        let changed = tx.execute(
            "DELETE FROM board_objects WHERE board_id=?1 AND id=?2 AND version=?3 AND child_id IS NULL",
            params![board, object.id, object.version],
        )?;
        if changed != 1 {
            return Err("文字方塊已變更，請重新開啟白板。".into());
        }
        tx.commit()?;
        Ok(())
    }

    /// Restores the same object identity and reports whether its prior position was available.
    pub(crate) fn restore_board_text(&mut self, board: i64, removed: &RemovedText) -> Result<bool> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if latest_removed(&tx, board)?.as_ref() != Some(removed) {
            return Err("最近移除的文字方塊已變更，請重新開啟選單。".into());
        }
        let preferred = (removed.x, removed.y);
        let (x, y) = position(&tx, board, Some(preferred))?;
        tx.execute(
            "INSERT INTO board_objects(id,board_id,body,x,y,version)
             SELECT object_id,board_id,body,?2,?3,version+1 FROM board_text_recovery WHERE board_id=?1",
            params![board, x, y],
        )?;
        tx.execute("DELETE FROM board_text_recovery WHERE board_id=?1", [board])?;
        tx.commit()?;
        Ok((x, y) == preferred)
    }
    pub(crate) fn move_board_object(&self,board:i64,o:&Object)->Result<()> {
        // A native mouse-up can reach here without movement. Do not make a
        // persisted layout-undo receipt stale merely for a click.
        if self.connection.execute("UPDATE board_objects SET x=?1,y=?2,
            version=CASE WHEN x<>?1 OR y<>?2 THEN version+1 ELSE version END
            WHERE board_id=?3 AND id=?4 AND version=?5",
            params![o.x,o.y,board,o.id,o.version])?!=1{return Err("物件已變更，請重新開啟白板。".into());}
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn card_creates_child_with_shared_note_and_parent_tile_survives_restore() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let result = (|| -> Result<()> {
            let live = dir.join("工作區.sqlite3");
            let backup = dir.join("備份.sqlite3");
            let restored = dir.join("隔離.sqlite3");
            let mut db = Notebook::open(&live, false)?;
            let parent = db.new_child_board(DEFAULT_BOARD_ID, "分鏡")?;
            let other = db.create_board("參考")?;
            let note = db.add(None, "鏡頭", "來源正文")?;
            let decoy = db.add(None, "鏡頭", "同名但不同正文")?;
            db.add_card_on(parent, note)?;
            db.add_card_on(other, note)?;
            db.add_card_on(parent, decoy)?;
            db.move_card_on(parent, note, 4210, 5010)?;
            let expected = (4210, 5010);

            let titles_before = db.list_boards()?.len();
            assert!(db.new_child_board_from_card(parent, note, (0, 0), "位置已變").is_err());
            db.trash(note)?;
            assert!(db.new_child_board_from_card(parent, note, expected, "垃圾桶").is_err());
            db.restore(note)?;
            assert!(db.new_child_board_from_card(parent, note, expected, "參考").is_err());
            db.connection.execute_batch(
                "CREATE TEMP TRIGGER reject_child_placement BEFORE INSERT ON board_cards
                 BEGIN SELECT RAISE(ABORT,'fixture refuses the late placement'); END;",
            )?;
            let error = db.new_child_board_from_card(parent, note, expected, "晚期失敗").unwrap_err();
            assert!(error.to_string().contains("fixture refuses the late placement"));
            assert_eq!(db.list_boards()?.len(), titles_before);
            assert!(db.objects_on(parent)?.is_empty());
            db.connection.execute_batch("DROP TRIGGER reject_child_placement;")?;

            let child = db.new_child_board_from_card(parent, note, expected, "近景")?;
            let (found, tile) = db.parent_board_tile(child)?.unwrap();
            assert_eq!(found.id, parent);
            assert_eq!(db.child_board_from_tile(parent, tile, child)?.id, child);
            assert_eq!(db.cards_on(child)?.into_iter().map(|card| card.id).collect::<Vec<_>>(), [note]);
            assert!(!db.cards_on(child)?.iter().any(|card| card.id == decoy));
            assert_eq!(db.cards_on(parent)?.into_iter().find(|card| card.id == note).map(|card| (card.x, card.y)), Some(expected));
            assert_eq!(db.cards_on(other)?[0].id, note);
            assert_eq!(db.board_path(child)?.into_iter().map(|board| board.id).collect::<Vec<_>>(),
                [DEFAULT_BOARD_ID, parent, child]);
            let grandchild = db.new_child_board(child, "更近")?;
            assert_eq!(db.parent_board_tile(grandchild)?.unwrap().0.id, child);
            let mut object = db.objects_on(parent)?.into_iter().find(|object| object.id == tile).unwrap();
            object.x = 4900;
            object.y = 7300;
            db.move_board_object(parent, &object)?;
            let saved = db.note(note)?.unwrap();
            db.save(note, saved.version, "主鏡頭", "編輯後全板同一正文")?;
            db.backup(&backup)?;
            drop(db);

            std::fs::copy(&backup, &restored)?;
            for path in [&live, &restored] {
                let mut db = Notebook::open(path, false)?;
                let (found, current_tile) = db.parent_board_tile(child)?.unwrap();
                assert_eq!((found.id, current_tile), (parent, tile));
                assert_eq!(db.objects_on(parent)?.into_iter().find(|object| object.id == tile)
                    .map(|object| (object.x, object.y)), Some((4900, 7300)));
                assert_eq!(db.cards_on(child)?[0].id, note);
                assert_eq!(db.note(note)?.unwrap().body, "編輯後全板同一正文");
                db.reparent_board(child, Some(parent), Some(other))?;
                assert!(db.child_board_from_tile(parent, tile, child).is_err());
                assert_eq!(db.parent_board_tile(child)?.unwrap().0.id, other);
                assert_eq!(db.cards_on(child)?[0].id, note);
            }
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(dir);
        result
    }

    use super::*;

    #[test]
    fn mixed_selection_moves_atomically_and_preserves_identity_edges_and_offsets() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let backup = dir.join("backup.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let board = db.create_board("多選測試")?;
        let a = db.add(None, "拖曳的卡片", "原始正文")?;
        let b = db.add(None, "未選取的卡片", "")?;
        db.add_card_on(board, a)?;
        db.add_card_on(board, b)?;
        db.move_card_on(board, a, 16, 16)?;
        db.move_card_on(board, b, 16, 286)?;
        db.add_edge_on(board, a, b)?;
        let group = db.create_board_group(board, "保留分區")?;
        db.set_board_group(board, a, None, Some(group))?;
        let child = db.new_child_board(board, "保留子白板")?;
        let text_id = db.add_board_text(board, "繁中備註\n保留內容")?;
        for mut object in db.objects_on(board)? {
            (object.x, object.y) = (if object.id == text_id { 276 } else { 536 }, 16);
            db.move_board_object(board, &object)?;
        }
        let card_position = |x, y| BoardPosition { item: BoardItem::Card(a), x, y, version: None };
        let snapshot = |db: &Notebook, x, y| -> Result<Vec<BoardPosition>> {
            let mut entries = vec![card_position(x, y)];
            entries.extend(db.objects_on(board)?.iter().map(|object| BoardPosition {
                item: BoardItem::Object(object.id), x: object.x, y: object.y, version: Some(object.version),
            }));
            Ok(entries)
        };
        let expected = snapshot(&db, 16, 16)?;
        assert!(db.move_board_selection(board, &expected, -17, 0).is_err());
        assert!(db.move_board_selection(board, &expected, 0, 270).is_err());
        db.connection.execute_batch("CREATE TEMP TRIGGER reject_object_move BEFORE UPDATE ON board_objects
            BEGIN SELECT RAISE(ABORT,'synthetic move failure'); END;")?;
        assert!(db.move_board_selection(board, &expected, 0, 135).is_err());
        assert_eq!((db.cards_on(board)?[0].x, db.cards_on(board)?[0].y), (16, 16));
        assert_eq!(snapshot(&db, 16, 16)?, expected);
        db.connection.execute_batch("DROP TRIGGER reject_object_move")?;

        let other = Notebook::open(&path, false)?;
        let latest_text = other.objects_on(board)?.into_iter().find(|entry| entry.id == text_id).unwrap();
        other.edit_board_text(board, &latest_text, "另一個視窗更新文字")?;
        assert!(db.move_board_selection(board, &expected, 0, 135).is_err());
        let fresh = snapshot(&db, 16, 16)?;
        db.move_board_selection(board, &fresh, 0, 135)?;
        assert!(db.move_board_selection(board, &fresh, 0, 135).is_err());
        let moved = snapshot(&db, 16, 151)?;
        db.trash(b)?;
        assert!(db.move_board_selection(board, &moved, 0, 135).is_err());
        db.restore(b)?;
        db.backup(&backup)?;
        drop(other);
        drop(db);

        let reopened = Notebook::open(&backup, false)?;
        assert_eq!(snapshot(&reopened, 16, 151)?, moved);
        let cards = reopened.cards_on(board)?;
        assert_eq!(cards.iter().map(|c| (c.id, c.x, c.y)).collect::<Vec<_>>(), [(a, 16, 151), (b, 16, 286)]);
        let positions = reopened.objects_on(board)?;
        assert_eq!(positions.iter().map(|entry| (entry.x, entry.y)).collect::<Vec<_>>(), [(536, 151), (276, 151)]);
        assert_eq!(positions[1].text, "另一個視窗更新文字");
        assert_eq!(reopened.edges_on(board)?.len(), 1);
        assert_eq!(reopened.groups_on(board)?[0].members, [a]);
        assert_eq!(reopened.parent_board(child)?, Some(board));
        assert_eq!(reopened.note(a)?.unwrap().body, "原始正文");
        drop(reopened);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn nested_text_and_note_placement_survive_reparent_and_backup()->Result<()>{
        let dir=super::super::tests::data_dir();
        let mut db=Notebook::open(&dir.join("workspace.sqlite"),false)?;
        let outer=db.new_child_board(1,"研究")?;
        let inner=db.new_child_board(outer,"參考")?;
        let path = db.board_path(inner)?;
        assert_eq!(path.iter().map(|board| board.id).collect::<Vec<_>>(), [1, outer, inner]);
        assert_eq!(path.iter().map(|board| board.title.as_str()).collect::<Vec<_>>(), ["白板", "研究", "參考"]);
        let text=db.add_board_text(inner,"白板文字\n第二行")?;
        let note=db.add(None,"筆記","保留筆記本文")?;
        db.add_card_on(inner,note)?;
        let objects=db.objects_on(inner)?;
        let mut original=objects[0].clone();
        assert_eq!(original.id,text);
        assert_eq!(original.child,None);
        let cards=db.cards_on(inner)?;
        assert_ne!((cards[0].x,cards[0].y),(original.x,original.y));
        db.edit_board_text(inner,&original,"更新文字")?;
        assert!(db.remove_board_text(inner,&original,None).is_err());
        let disposable=db.add_board_text(inner,"待移除")?;
        let disposable=db.objects_on(inner)?.into_iter().find(|o|o.id==disposable).unwrap();
        assert!(db.remove_board_text(outer,&disposable,None).is_err());
        let child_tile = db.objects_on(outer)?.remove(0);
        assert!(db.remove_board_text(outer,&child_tile,None).is_err());
        db.remove_board_text(inner,&disposable,None)?;
        assert!(!db.objects_on(inner)?.iter().any(|o|o.id==disposable.id));
        original.x=125;
        assert!(db.move_board_object(inner,&original).is_err());
        let mut current=db.objects_on(inner)?.remove(0);
        current.x=91;current.y=152;
        db.move_board_object(inner,&current)?;
        assert!(db.reparent_board(outer,Some(1),Some(inner)).is_err());
        assert!(db.reparent_board(inner,None,Some(1)).is_err());
        let tile=db.objects_on(outer)?.remove(0);
        db.reparent_board(inner,Some(outer),Some(1))?;
        assert!(db.objects_on(outer)?.is_empty());
        assert!(db.objects_on(1)?.iter().any(|o|o.id==tile.id && o.child==Some(inner)));
        let backup=dir.join("copy.sqlite");
        db.backup(&backup)?;
        drop(db);
        let mut copy=Notebook::open(&backup,false)?;
        assert_eq!(copy.parent_board(inner)?,Some(1));
        assert_eq!(copy.board_path(inner)?.iter().map(|board| board.id).collect::<Vec<_>>(), [1, inner]);
        let object=copy.objects_on(inner)?.remove(0);
        assert_eq!((object.id,object.text,object.x,object.y),(text,"更新文字".to_owned(),91,152));
        assert_eq!(copy.note(note)?.unwrap().body,"保留筆記本文");
        copy.reparent_board(inner,Some(1),None)?;
        assert_eq!(copy.parent_board(inner)?,None);
        assert_eq!(copy.board_path(inner)?.iter().map(|board| board.id).collect::<Vec<_>>(), [inner]);
        assert_eq!(copy.objects_on(inner)?.len(),1);
        drop(copy);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn restores_latest_text_after_reopen_and_backup_without_displacing_cards() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("workspace.sqlite");
        let mut db = Notebook::open(&path, false)?;
        let board = db.new_child_board(1, "分鏡")?;
        let id = db.add_board_text(board, "繁中旁註\n第二行")?;
        let mut moved = db.objects_on(board)?.remove(0);
        (moved.x, moved.y) = (123, 321);
        db.move_board_object(board, &moved)?;
        let object = db.objects_on(board)?.remove(0);
        db.remove_board_text(board, &object, None)?;
        let removed = db.latest_removed_board_text(board)?.unwrap();
        assert!(db.latest_removed_board_text(1)?.is_none());
        assert!(db.restore_board_text(1, &removed).is_err());
        drop(db);

        let mut db = Notebook::open(&path, false)?;
        assert!(db.restore_board_text(board, &removed)?);
        assert!(db.restore_board_text(board, &removed).is_err());
        let current = db.objects_on(board)?.remove(0);
        assert_eq!(current.id, id);
        assert_eq!((current.x, current.y), (object.x, object.y));
        assert_eq!(current.text, object.text);
        assert_eq!(current.version, object.version + 1);
        assert!(db.remove_board_text(board, &object, None).is_err());

        db.remove_board_text(board, &current, None)?;
        let note = db.add(None, "白板的另一張卡", "保留")?;
        db.add_card_on(board, note)?;
        db.move_card_on(board, note, current.x + 1, current.y + 1)?;
        let card = db.cards_on(board)?.remove(0);
        assert_eq!((card.x, card.y), (current.x + 1, current.y + 1));
        let backup = dir.join("backup.sqlite");
        db.backup(&backup)?;
        drop(db);

        let mut copy = Notebook::open(&backup, false)?;
        let removed = copy.latest_removed_board_text(board)?.unwrap();
        assert!(!copy.restore_board_text(board, &removed)?);
        let card_after = copy.cards_on(board)?.remove(0);
        let restored = copy.objects_on(board)?.remove(0);
        assert_eq!((card_after.id, card_after.x, card_after.y), (note, card.x, card.y));
        assert_eq!(restored.text, current.text);
        assert!((restored.x-card.x).abs() >= 232 || (restored.y-card.y).abs() >= 114);
        assert!(copy.latest_removed_board_text(board)?.is_none());
        drop(copy);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn old_confirmation_and_full_board_keep_the_current_recovery_entry() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("workspace.sqlite");
        let mut db = Notebook::open(&path, false)?;
        let board = 1;
        db.add_board_text(board, "先移除")?;
        db.add_board_text(board, "第二筆")?;
        db.add_board_text(board, "保留")?;
        let objects = db.objects_on(board)?;
        db.remove_board_text(board, &objects[0], None)?;
        let old_confirmation = db.latest_removed_board_text(board)?.unwrap();
        let mut other = Notebook::open(&path, false)?;
        other.remove_board_text(board, &objects[1], Some(&old_confirmation))?;
        assert!(db.remove_board_text(board, &objects[2], Some(&old_confirmation)).is_err());
        assert!(db.restore_board_text(board, &old_confirmation).is_err());
        let current = db.latest_removed_board_text(board)?.unwrap();
        assert_eq!(current.text, "第二筆");
        assert_eq!(db.objects_on(board)?[0].id, objects[2].id);

        for index in 1..LIMIT {
            db.add_board_text(board, &format!("物件 {index}"))?;
        }
        assert!(db.restore_board_text(board, &current).is_err());
        assert_eq!(db.latest_removed_board_text(board)?, Some(current));
        assert_eq!(db.objects_on(board)?.len() as i64, LIMIT);
        drop(other);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn limits_reject_without_orphaning_child_or_descendants()->Result<()> {
        let dir=super::super::tests::data_dir();
        let mut db=Notebook::open(&dir.join("workspace.sqlite"),false)?;
        let mut current=1;
        for level in 1..=MAX_DEPTH { current=db.new_child_board(current,&format!("第 {level} 層"))?; }
        assert_eq!(db.board_path(current)?.len(), MAX_DEPTH + 1);
        assert!(db.board_path(-1).is_err());
        let count=db.list_boards()?.len();
        assert!(db.new_child_board(current,"太深").is_err());
        assert_eq!(db.list_boards()?.len(),count);
        let detached=db.create_board("獨立白板")?;
        let leaf=db.new_child_board(detached,"葉節點")?;
        assert!(db.reparent_board(detached,None,Some(current)).is_err());
        assert_eq!(db.parent_board(detached)?,None);
        assert_eq!(db.parent_board(leaf)?,Some(detached));
        assert!(db.add_board_text(leaf,"   ").is_err());
        assert!(db.add_board_text(leaf,&"a".repeat(TEXT_LIMIT+1)).is_err());
        for i in 0..LIMIT { db.add_board_text(leaf,&i.to_string())?; }
        let count=db.list_boards()?.len();
        assert!(db.new_child_board(leaf,"無空間").is_err());
        assert_eq!(db.list_boards()?.len(),count);
        assert_eq!(db.objects_on(leaf)?.len(),LIMIT as usize);
        drop(db);std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
