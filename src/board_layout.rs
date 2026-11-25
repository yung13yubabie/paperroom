//! Geometry-based board arrangements and a persistent one-step undo.
use super::board_images::ImageSize;
use super::board_objects::{BoardItem, BoardPosition};
use super::{Notebook, Result};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use std::collections::HashSet;

pub(super) const SCHEMA: &str = "
CREATE TABLE board_layout_undo(
    board_id INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
    object INTEGER NOT NULL CHECK(object IN (0,1)),
    item_id INTEGER NOT NULL,
    old_x INTEGER NOT NULL CHECK(old_x BETWEEN 0 AND 10000),
    old_y INTEGER NOT NULL CHECK(old_y BETWEEN 0 AND 50000),
    new_x INTEGER NOT NULL CHECK(new_x BETWEEN 0 AND 10000),
    new_y INTEGER NOT NULL CHECK(new_y BETWEEN 0 AND 50000),
    width INTEGER NOT NULL CHECK(width BETWEEN 64 AND 768),
    height INTEGER NOT NULL CHECK(height BETWEEN 64 AND 768),
    new_version INTEGER,
    CHECK((object=1)=(new_version IS NOT NULL)),
    PRIMARY KEY(board_id,object,item_id)
) WITHOUT ROWID;
CREATE TRIGGER invalidate_arranged_card_delete AFTER DELETE ON board_cards
WHEN EXISTS(SELECT 1 FROM board_layout_undo WHERE board_id=OLD.board_id AND object=0 AND item_id=OLD.note_id)
BEGIN
    DELETE FROM board_layout_undo WHERE board_id=OLD.board_id;
END;
CREATE TRIGGER invalidate_arranged_card_move AFTER UPDATE OF x,y,image_width,image_height ON board_cards
WHEN (OLD.x IS NOT NEW.x OR OLD.y IS NOT NEW.y OR OLD.image_width IS NOT NEW.image_width
      OR OLD.image_height IS NOT NEW.image_height)
 AND EXISTS(SELECT 1 FROM board_layout_undo WHERE board_id=OLD.board_id AND object=0 AND item_id=OLD.note_id)
BEGIN
    DELETE FROM board_layout_undo WHERE board_id=OLD.board_id;
END;
CREATE TRIGGER invalidate_arranged_object_delete AFTER DELETE ON board_objects
WHEN EXISTS(SELECT 1 FROM board_layout_undo WHERE board_id=OLD.board_id AND object=1 AND item_id=OLD.id)
BEGIN
    DELETE FROM board_layout_undo WHERE board_id=OLD.board_id;
END;
";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoardLayoutRect {
    pub position: BoardPosition,
    pub size: ImageSize,
}

#[derive(Clone, Copy)]
pub(crate) enum BoardArrangement {
    Left,
    CenterX,
    Right,
    Top,
    CenterY,
    Bottom,
    GapX,
    GapY,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoardLayoutChange {
    old: (i32, i32),
    after: BoardLayoutRect,
}

impl BoardLayoutRect {
    fn overlaps(self, other: Self) -> bool {
        let (a, b) = (self.position, other.position);
        a.x < b.x + other.size.width && b.x < a.x + self.size.width
            && a.y < b.y + other.size.height && b.y < a.y + self.size.height
    }

    fn at(self, x: i32, y: i32) -> Self {
        Self { position: BoardPosition { x, y, ..self.position }, ..self }
    }
}

fn item(id: i64, object: bool) -> BoardItem {
    if object { BoardItem::Object(id) } else { BoardItem::Card(id) }
}

fn item_key(item: BoardItem) -> (bool, i64) {
    match item { BoardItem::Card(id) => (false, id), BoardItem::Object(id) => (true, id) }
}

fn arrange(expected: &[BoardLayoutRect], operation: BoardArrangement) -> Result<Vec<BoardLayoutRect>> {
    use BoardArrangement::*;
    let horizontal = matches!(operation, Left | CenterX | Right | GapX);
    let distribute = matches!(operation, GapX | GapY);
    let minimum = if distribute { 3 } else { 2 };
    if expected.len() < minimum {
        return Err("對齊至少需選取 2 個物件，等距排列至少需選取 3 個。".into());
    }
    let start = |rect: &BoardLayoutRect| if horizontal { rect.position.x } else { rect.position.y };
    let length = |rect: &BoardLayoutRect| if horizontal { rect.size.width } else { rect.size.height };
    let min = expected.iter().map(start).min().ok_or("沒有選取的物件。")?;
    let max = expected.iter().map(|rect| start(rect) + length(rect)).max().ok_or("沒有選取的物件。")?;
    let mut result = expected.to_vec();
    if distribute {
        let free = max - min - result.iter().map(length).sum::<i32>();
        if free < 0 {
            return Err("選取範圍無法容納等距排列。請先拉開最外側物件。".into());
        }
        let separators = result.len() as i32 - 1;
        let (gap, leftover) = (free / separators, free % separators);
        result.sort_unstable_by_key(|rect| (start(rect), item_key(rect.position.item)));
        let mut cursor = min;
        for (index, rect) in result.iter_mut().enumerate() {
            if horizontal { rect.position.x = cursor; } else { rect.position.y = cursor; }
            cursor += length(rect) + gap + i32::from((index as i32) < leftover);
        }
    } else {
        for rect in &mut result {
            let coordinate = match operation {
                Left | Top => min,
                CenterX | CenterY => (min + max - length(rect)) / 2,
                Right | Bottom => max - length(rect),
                _ => unreachable!(),
            };
            if horizontal { rect.position.x = coordinate; } else { rect.position.y = coordinate; }
        }
    }
    Ok(result)
}

fn read_layout(db: &Connection, board: i64) -> Result<Vec<(BoardLayoutRect, bool)>> {
    let mut query = db.prepare(
        "SELECT 0,c.note_id,c.x,c.y,NULL,c.width,c.height,n.trash_root IS NULL
         FROM board_card_geometry c JOIN notes n ON n.id=c.note_id WHERE c.board_id=?1
         UNION ALL SELECT 1,id,x,y,version,232,114,1 FROM board_objects WHERE board_id=?1",
    )?;
    let rows = query.query_map([board], |row| Ok((BoardLayoutRect {
        position: BoardPosition { item: item(row.get(1)?, row.get(0)?), x: row.get(2)?, y: row.get(3)?, version: row.get(4)? },
        size: ImageSize { width: row.get(5)?, height: row.get(6)? },
    }, row.get(7)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn validate_expected(layout: &[(BoardLayoutRect, bool)], expected: &[BoardLayoutRect]) -> Result<HashSet<BoardItem>> {
    let selected = expected.iter().map(|entry| entry.position.item).collect::<HashSet<_>>();
    if expected.is_empty() || expected.len() > 256 || selected.len() != expected.len() {
        return Err("白板選取無效。請重新選取。".into());
    }
    if expected.iter().any(|entry| !layout.iter().any(|(actual, active)| *active && actual == entry)) {
        return Err("選取物件、尺寸或位置已變更，請重新整理後重試。".into());
    }
    Ok(selected)
}

fn validate_destination(
    target: &[BoardLayoutRect], layout: &[(BoardLayoutRect, bool)], selected: &HashSet<BoardItem>,
    arrange_selected: bool,
) -> Result<()> {
    for (index, rect) in target.iter().enumerate() {
        if !(0..=10000).contains(&rect.position.x) || !(0..=50000).contains(&rect.position.y) {
            return Err("排列會超出白板範圍，沒有移動任何物件。".into());
        }
        if layout.iter().any(|(other, _)| !selected.contains(&other.position.item) && rect.overlaps(*other)) {
            return Err("目的位置與其他物件相疊，沒有移動任何物件。".into());
        }
        if arrange_selected && target[..index].iter().any(|prior| rect.overlaps(*prior)) {
            return Err("這個排列會讓選取的物件相疊，沒有移動任何物件。".into());
        }
    }
    Ok(())
}

fn write_layout(
    tx: &Transaction<'_>, board: i64, expected: &[BoardLayoutRect], target: &[BoardLayoutRect],
) -> Result<Vec<BoardLayoutChange>> {
    let mut cards = tx.prepare("UPDATE board_cards SET x=?3,y=?4 WHERE board_id=?1 AND note_id=?2")?;
    let mut objects = tx.prepare("UPDATE board_objects SET x=?3,y=?4,version=?5 WHERE board_id=?1 AND id=?2")?;
    let mut changes = Vec::with_capacity(target.len());
    for next in target {
        let mut after = *next;
        let old = expected.iter().find(|entry| entry.position.item == after.position.item)
            .ok_or("白板排列資料已變更。")?.position;
        let coordinates = (old.x, old.y);
        if coordinates != (after.position.x, after.position.y) {
            let (x, y) = (after.position.x, after.position.y);
            let changed = match old.item {
                BoardItem::Card(note) => cards.execute(params![board, note, x, y])?,
                BoardItem::Object(id) => {
                    let version = old.version.and_then(|version| version.checked_add(1)).ok_or("物件版本無效。")?;
                    after.position.version = Some(version);
                    objects.execute(params![board, id, x, y, version])?
                }
            };
            if changed != 1 {
                return Err("白板物件已變更，沒有移動任何物件。".into());
            }
        }
        changes.push(BoardLayoutChange { old: coordinates, after });
    }
    Ok(changes)
}

fn read_undo(db: &Connection, board: i64) -> Result<Vec<BoardLayoutChange>> {
    let mut query = db.prepare(
        "SELECT object,item_id,old_x,old_y,new_x,new_y,width,height,new_version
         FROM board_layout_undo WHERE board_id=?1 ORDER BY object,item_id",
    )?;
    let rows = query.query_map([board], |row| Ok(BoardLayoutChange {
        old: (row.get(2)?, row.get(3)?),
        after: BoardLayoutRect {
            position: BoardPosition { item: item(row.get(1)?, row.get(0)?),
                x: row.get(4)?, y: row.get(5)?, version: row.get(8)? },
            size: ImageSize { width: row.get(6)?, height: row.get(7)? },
        },
    }))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

impl Notebook {
    pub(crate) fn arrange_board_selection(
        &self, board: i64, expected: &[BoardLayoutRect], operation: BoardArrangement,
    ) -> Result<usize> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let layout = read_layout(&tx, board)?;
        let selected = validate_expected(&layout, expected)?;
        let target = arrange(expected, operation)?;
        validate_destination(&target, &layout, &selected, true)?;
        let changes = write_layout(&tx, board, expected, &target)?;
        let moved = changes.iter().filter(|c| c.old != (c.after.position.x, c.after.position.y)).count();
        if moved != 0 {
            tx.execute("DELETE FROM board_layout_undo WHERE board_id=?1", [board])?;
            let mut save = tx.prepare(
                "INSERT INTO board_layout_undo(board_id,object,item_id,old_x,old_y,new_x,new_y,width,height,new_version)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            )?;
            for change in changes {
                let (p, size) = (change.after.position, change.after.size);
                let (object, id) = item_key(p.item);
                save.execute(params![board, object, id, change.old.0, change.old.1, p.x, p.y, size.width, size.height, p.version])?;
            }
        }
        tx.commit()?;
        Ok(moved)
    }

    pub(crate) fn board_layout_undo(&self, board: i64) -> Result<Vec<BoardLayoutChange>> {
        read_undo(&self.connection, board)
    }

    pub(crate) fn undo_board_layout(&self, board: i64, confirmed: &[BoardLayoutChange]) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let undo = read_undo(&tx, board)?;
        if undo.is_empty() || undo != confirmed {
            return Err("最近一次排列已變更。請重新確認復原。".into());
        }
        let layout = read_layout(&tx, board)?;
        let expected = undo.iter().map(|change| change.after).collect::<Vec<_>>();
        let selected = validate_expected(&layout, &expected)?;
        let destination = undo.iter().map(|change| change.after.at(change.old.0, change.old.1)).collect::<Vec<_>>();
        // The old arrangement may include overlap within the same selection.
        validate_destination(&destination, &layout, &selected, false)?;
        write_layout(&tx, board, &expected, &destination)?;
        tx.execute("DELETE FROM board_layout_undo WHERE board_id=?1", [board])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::board_images::BoardImageInput;
    use image::ImageEncoder;

    fn frame(item: BoardItem, x: i32, y: i32, width: i32, height: i32) -> BoardLayoutRect {
        BoardLayoutRect {
            position: BoardPosition { item, x, y, version: None },
            size: ImageSize { width, height },
        }
    }

    #[test]
    fn mixed_gap_and_center_geometry_uses_measured_edges_not_card_defaults() -> Result<()> {
        use BoardArrangement::*;
        let entries = [frame(BoardItem::Card(1), 16, 16, 232, 114),
            frame(BoardItem::Card(2), 260, 170, 128, 100),
            frame(BoardItem::Object(1), 600, 340, 232, 114)];
        let coords = |mode| Ok::<_, Box<dyn std::error::Error>>(arrange(&entries, mode)?
            .iter().map(|frame| (frame.position.x, frame.position.y)).collect::<Vec<_>>());
        assert_eq!(coords(GapX)?, [(16, 16), (360, 170), (600, 340)]);
        assert_eq!(coords(GapY)?, [(16, 16), (260, 185), (600, 340)]);
        assert_eq!(coords(Left)?, [(16, 16), (16, 170), (16, 340)]);
        assert_eq!(coords(CenterX)?, [(308, 16), (360, 170), (308, 340)]);
        assert_eq!(coords(Right)?, [(600, 16), (704, 170), (600, 340)]);
        assert_eq!(coords(Top)?, [(16, 16), (260, 16), (600, 16)]);
        assert_eq!(coords(CenterY)?, [(16, 178), (260, 185), (600, 178)]);
        assert_eq!(coords(Bottom)?, [(16, 340), (260, 354), (600, 340)]);
        let too_narrow = entries.map(|entry| entry.at(10, entry.position.y));
        assert!(arrange(&too_narrow, GapX).is_err());
        assert!(arrange(&entries[..2], GapX).is_err());
        Ok(())
    }

    #[test]
    fn clicking_an_aligned_text_box_does_not_invalidate_persisted_undo() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("offline-undo.sqlite3");
        let (board, anchor, text) = {
            let mut db = Notebook::open(&path, false)?;
            let board = db.create_board("離線復原")?;
            let anchor = db.add(None, "定位卡", "")?;
            db.add_card_on(board, anchor)?;
            db.move_card_on(board, anchor, 16, 16)?;
            let text = db.add_board_text(board, "保留文字")?;
            let mut object = db.objects_on(board)?.remove(0);
            (object.x, object.y) = (480, 170);
            db.move_board_object(board, &object)?;
            let expected = read_layout(&db.connection, board)?.into_iter().map(|(rect, _)| rect).collect::<Vec<_>>();
            assert_eq!(db.arrange_board_selection(board, &expected, BoardArrangement::Left)?, 1);
            (board, anchor, text)
        };
        {
            let db = Notebook::open(&path, false)?;
            let receipt = db.board_layout_undo(board)?;
            let clicked = db.objects_on(board)?.remove(0);
            assert_eq!((clicked.id, clicked.x, clicked.y), (text, 16, 170));
            // Native mouse-up for a text box uses this method even for a click with no drag.
            db.move_board_object(board, &clicked)?;
            assert_eq!(db.objects_on(board)?[0].version, clicked.version);
            db.undo_board_layout(board, &receipt)?;
            let restored = db.objects_on(board)?.remove(0);
            assert_eq!((restored.id, restored.x, restored.y, restored.text.as_str()), (text, 480, 170, "保留文字"));
            assert_eq!(restored.version, clicked.version + 1);
            assert_eq!((db.cards_on(board)?[0].id, db.cards_on(board)?[0].x), (anchor, 16));
            assert!(db.board_layout_undo(board)?.is_empty());

            let expected = read_layout(&db.connection, board)?.into_iter().map(|(rect, _)| rect).collect::<Vec<_>>();
            assert_eq!(db.arrange_board_selection(board, &expected, BoardArrangement::Left)?, 1);
            let receipt = db.board_layout_undo(board)?;
            let mut moved = db.objects_on(board)?.remove(0);
            moved.x = 600;
            db.move_board_object(board, &moved)?;
            assert!(db.undo_board_layout(board, &receipt).is_err());
            assert!(db.move_board_object(board, &moved).is_err()); // stale with current coordinates
            let last = db.objects_on(board)?.remove(0);
            assert_eq!((last.x, last.version), (600, moved.version + 1));
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn arrangement_and_undo_survive_v21_migration_backup_and_refuse_collisions_or_stale_receipts() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("whiteboard.sqlite");
        let backup = dir.join("backup.sqlite");
        let (board, note, image, text) = {
            let mut db = Notebook::open(&path, false)?;
            let board = db.create_board("測試排列")?;
            let note = db.add(None, "主筆記", "保留")?;
            db.add_card_on(board, note)?;
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png).write_image(
                &[10, 20, 30, 255], 1, 1, image::ExtendedColorType::Rgba8,
            )?;
            let image = db.add_board_images(board, &[BoardImageInput::new("主圖片.png".into(), png)?])?[0];
            db.move_card_on(board, image, 260, 170)?;
            db.resize_board_image(board, image, (260, 170), ImageSize::IMAGE,
                ImageSize { width: 128, height: 100 })?;
            let text = db.add_board_text(board, "保留文字")?;
            let mut object = db.objects_on(board)?.remove(0);
            (object.x, object.y) = (600, 340);
            db.move_board_object(board, &object)?;
            db.add_edge_on(board, note, image)?;
            super::super::tests::remove_v22_schema(&db.connection)?;
            db.connection.pragma_update(None, "user_version", 21)?;
            (board, note, image, text)
        };

        let db = Notebook::open(&path, false)?;
        let snapshot = |db: &Notebook| -> Result<Vec<BoardLayoutRect>> {
            let members = [BoardItem::Card(note), BoardItem::Card(image), BoardItem::Object(text)];
            let mut entries: Vec<_> = read_layout(&db.connection, board)?.into_iter().filter_map(|(entry, active)| {
                (active && members.contains(&entry.position.item)).then_some(entry)
            }).collect();
            entries.sort_unstable_by_key(|entry| item_key(entry.position.item));
            Ok(entries)
        };
        let original = snapshot(&db)?;
        assert_eq!(original.len(), 3);
        assert!(db.board_layout_undo(board)?.is_empty());
        db.resize_board_image(board, image, (260, 170), ImageSize { width: 128, height: 100 },
            ImageSize { width: 129, height: 100 })?;
        assert!(db.arrange_board_selection(board, &original, BoardArrangement::GapX).is_err());
        db.resize_board_image(board, image, (260, 170), ImageSize { width: 129, height: 100 },
            ImageSize { width: 128, height: 100 })?;
        db.connection.execute_batch("CREATE TEMP TRIGGER fail_receipt BEFORE INSERT ON board_layout_undo
            BEGIN SELECT RAISE(ABORT, 'simulated undo write failure'); END;")?;
        assert!(db.arrange_board_selection(board, &original, BoardArrangement::GapX).is_err());
        assert_eq!(snapshot(&db)?, original);
        assert!(db.board_layout_undo(board)?.is_empty());
        db.connection.execute_batch("DROP TRIGGER fail_receipt")?;
        assert_eq!(db.arrange_board_selection(board, &original, BoardArrangement::GapX)?, 1);
        let arranged = snapshot(&db)?;
        assert_eq!(arranged[1].position.x, 360);
        let receipt = db.board_layout_undo(board)?;
        assert_eq!(receipt.len(), 3);
        db.backup(&backup)?;
        drop(db);

        let mut reopened = Notebook::open(&backup, false)?;
        let blocker = reopened.add(None, "新物件佔了舊位置", "")?;
        reopened.add_card_on(board, blocker)?;
        reopened.move_card_on(board, blocker, 120, 170)?;
        assert!(reopened.undo_board_layout(board, &receipt).is_err());
        assert_eq!(snapshot(&reopened)?, arranged);
        assert_eq!(reopened.board_layout_undo(board)?, receipt);
        reopened.remove_card_on(board, blocker)?;
        reopened.undo_board_layout(board, &receipt)?;
        assert_eq!(snapshot(&reopened)?, original);
        assert!(reopened.board_layout_undo(board)?.is_empty());
        assert!(reopened.undo_board_layout(board, &receipt).is_err());
        assert_eq!(reopened.edges_on(board)?.len(), 1);
        assert_eq!(reopened.note(note)?.unwrap().body, "保留");
        let toward_left = snapshot(&reopened)?;
        assert_eq!(reopened.arrange_board_selection(board, &toward_left, BoardArrangement::Left)?, 2);
        assert!(reopened.undo_board_layout(board, &receipt).is_err());
        let last = snapshot(&reopened)?;
        assert_eq!(last.iter().map(|entry| entry.position.x).collect::<Vec<_>>(), [16, 16, 16]);
        let obsolete = reopened.board_layout_undo(board)?;
        reopened.remove_card_on(board, note)?;
        reopened.add_card_on(board, note)?;
        let original_place = last.iter().find(|entry| entry.position.item == BoardItem::Card(note)).unwrap();
        reopened.move_card_on(board, note, original_place.position.x, original_place.position.y)?;
        assert_eq!(snapshot(&reopened)?, last);
        assert!(reopened.board_layout_undo(board)?.is_empty());
        assert!(reopened.undo_board_layout(board, &obsolete).is_err());
        drop(reopened);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
