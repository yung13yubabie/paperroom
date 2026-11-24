use super::{Card, MAX_TITLE_CHARS, Notebook, Result};
use super::board_images::ImageSize;
use super::board_objects::{BoardItem, BoardPosition};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::{BTreeMap, HashSet};

pub const DEFAULT_BOARD_ID: i64 = 1;
const MAX_CARDS: i64 = 128;

pub struct Board {
    pub id: i64,
    pub title: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BoardEdge {
    pub from: i64,
    pub to: i64,
}

pub(super) const SCHEMA: &str = "
    CREATE TABLE boards(
        id INTEGER PRIMARY KEY,
        title TEXT NOT NULL COLLATE NOCASE UNIQUE
    );
    INSERT INTO boards(id,title) VALUES(1,'白板');
    ALTER TABLE board_cards RENAME TO single_board_cards;
    CREATE TABLE board_cards(
        board_id INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
        note_id INTEGER NOT NULL REFERENCES notes(id),
        x INTEGER NOT NULL CHECK(x BETWEEN 0 AND 10000),
        y INTEGER NOT NULL CHECK(y BETWEEN 0 AND 50000),
        PRIMARY KEY(board_id,note_id)
    );
    INSERT INTO board_cards(board_id,note_id,x,y)
        SELECT 1,note_id,x,y FROM single_board_cards;
    DROP TABLE single_board_cards;
    CREATE TABLE board_edges(
        board_id INTEGER NOT NULL,
        from_note INTEGER NOT NULL,
        to_note INTEGER NOT NULL CHECK(from_note < to_note),
        PRIMARY KEY(board_id,from_note,to_note),
        FOREIGN KEY(board_id,from_note) REFERENCES board_cards(board_id,note_id) ON DELETE CASCADE,
        FOREIGN KEY(board_id,to_note) REFERENCES board_cards(board_id,note_id) ON DELETE CASCADE
    );
";

pub(super) fn create_board_on(db: &rusqlite::Connection, title: &str) -> Result<i64> {
    let title = title.trim();
    if title.is_empty() || title.chars().any(char::is_control) || title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("白板名稱須為 1 至 {MAX_TITLE_CHARS} 個單行字元。").into());
    }
    let changed = db.execute(
        "INSERT INTO boards(title) VALUES(?1) ON CONFLICT(title) DO NOTHING", [title],
    )?;
    if changed == 0 {
        return Err("已有同名白板。".into());
    }
    Ok(db.last_insert_rowid())
}

pub(super) fn place_card_on(connection: &rusqlite::Connection, board: i64, note: i64) -> Result<()> {
    let (exists, active, placed, count): (bool, bool, bool, i64) = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM boards WHERE id=?1),
                EXISTS(SELECT 1 FROM notes WHERE id=?2 AND trash_root IS NULL),
                EXISTS(SELECT 1 FROM board_cards WHERE board_id=?1 AND note_id=?2),
                (SELECT count(*) FROM board_cards WHERE board_id=?1)",
        params![board, note],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    if !exists {
        return Err("找不到白板。".into());
    }
    if !active {
        return Err("找不到筆記，或筆記已移到垃圾桶。".into());
    }
    if placed {
        return Ok(());
    }
    if count >= MAX_CARDS {
        return Err("每張白板最多可放 128 張卡片。".into());
    }

    // Reuse a free grid position even after a card was moved or removed.
    let changed = connection.execute(
        "WITH RECURSIVE slots(slot,x,y) AS (
            VALUES(0,16,16)
            UNION ALL
            SELECT slot+1,16+((slot+1)%2)*260,16+((slot+1)/2)*135
            FROM slots WHERE slot<737
         ), occupied AS (
            SELECT x,y,width,height FROM board_card_geometry WHERE board_id=?1
            UNION ALL SELECT x,y,232,114 FROM board_objects WHERE board_id=?1
         )
         INSERT INTO board_cards(board_id,note_id,x,y)
         SELECT ?1,n.id,s.x,s.y FROM notes n
         LEFT JOIN board_image_cards i ON i.note_id=n.id CROSS JOIN slots s
         WHERE n.id=?2 AND n.trash_root IS NULL
         AND NOT EXISTS(
            SELECT 1 FROM occupied c WHERE c.x<s.x+232 AND c.x+c.width>s.x
              AND c.y<s.y+CASE WHEN i.note_id IS NULL THEN 114 ELSE 156 END AND c.y+c.height>s.y
         )
         ORDER BY s.slot LIMIT 1",
        params![board, note],
    )?;
    if changed != 1 {
        return Err("無法在白板上放入卡片。".into());
    }
    Ok(())
}

impl Notebook {
    pub fn list_boards(&self) -> Result<Vec<Board>> {
        let mut query = self.connection.prepare("SELECT id,title FROM boards ORDER BY id")?;
        let boards = query.query_map([], |row| {
            Ok(Board { id: row.get(0)?, title: row.get(1)? })
        })?;
        Ok(boards.collect::<rusqlite::Result<_>>()?)
    }

    pub fn create_board(&self, title: &str) -> Result<i64> {
        create_board_on(&self.connection, title)
    }

    pub fn cards_on(&self, board: i64) -> Result<Vec<Card>> {
        let mut query = self.connection.prepare(
            "SELECT n.id,n.title,substr(n.body,1,240),c.x,c.y,
                    CASE WHEN c.is_image THEN c.width END,c.height
             FROM board_card_geometry c JOIN notes n ON n.id=c.note_id
             WHERE c.board_id=?1 AND n.trash_root IS NULL ORDER BY c.note_id",
        )?;
        let cards = query.query_map([board], |row| {
            let image = match row.get(5)? {
                Some(width) => Some(ImageSize { width, height: row.get(6)? }),
                None => None,
            };
            Ok(Card {
                id: row.get(0)?,
                title: row.get(1)?,
                excerpt: row.get(2)?,
                x: row.get(3)?,
                y: row.get(4)?,
                image,
            })
        })?;
        Ok(cards.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_card_on(&mut self, board: i64, note: i64) -> Result<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        place_card_on(&tx, board, note)?;
        tx.commit()?;
        Ok(())
    }

    /// Adds references to the selected notes in another board with its own placement rules.
    /// The source placements and any existing destination placements remain unchanged.
    pub(crate) fn reuse_board_cards(
        &self, source: i64, destination: i64, expected: &[BoardPosition],
    ) -> Result<usize> {
        if source == destination || expected.is_empty() || expected.len() as i64 > MAX_CARDS {
            return Err("請選取卡片與另一張目的白板。".into());
        }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let mut selected = HashSet::with_capacity(expected.len());
        let mut added = 0;
        {
            let mut placements = tx.prepare(
                "SELECT EXISTS(SELECT 1 FROM board_cards c JOIN notes n ON n.id=c.note_id
                               WHERE c.board_id=?1 AND c.note_id=?3 AND c.x=?4 AND c.y=?5 AND n.trash_root IS NULL),
                        EXISTS(SELECT 1 FROM board_cards WHERE board_id=?2 AND note_id=?3)",
            )?;
            for position in expected {
                let BoardItem::Card(note) = position.item else {
                    return Err("這項操作支援筆記、圖片與 PDF 摘錄卡。請取消選取文字方塊和子白板。".into());
                };
                if !selected.insert(note) {
                    return Err("卡片選取重複，請重新選取。".into());
                }
                let (active, already_placed): (bool, bool) = placements.query_row(
                    params![source, destination, note, position.x, position.y],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                if !active {
                    return Err("來源卡片或位置已變更，沒有放入任何卡片。請重新整理並選取。".into());
                }
                if !already_placed {
                    place_card_on(&tx, destination, note)?;
                    added += 1;
                }
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// Move whole placements and their board-local relationships in one transaction.
    pub(crate) fn move_cards_between_boards(
        &self, source: i64, destination: i64, expected: &[BoardPosition], dx: i32, dy: i32,
    ) -> Result<usize> {
        if source == destination || expected.is_empty() || expected.len() as i64 > MAX_CARDS {
            return Err("請選取卡片與另一張目的白板。".into());
        }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let (exists, capacity): (bool, i64) = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM boards WHERE id=?1),
                    (SELECT count(*) FROM board_cards WHERE board_id=?1)",
            [destination], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if !exists {
            return Err("目的白板已不存在。".into());
        }
        if capacity + expected.len() as i64 > MAX_CARDS {
            return Err("目的白板的卡片空間不足，未搬移任何卡片。".into());
        }

        let mut selected = HashSet::with_capacity(expected.len());
        let mut moved = Vec::with_capacity(expected.len());
        {
            let mut check = tx.prepare(
                "WITH occupied AS (
                    SELECT x,y,width,height FROM board_card_geometry WHERE board_id=?2
                    UNION ALL SELECT x,y,232,114 FROM board_objects WHERE board_id=?2
                 )
                 SELECT EXISTS(SELECT 1 FROM board_cards c JOIN notes n ON n.id=c.note_id
                               WHERE c.board_id=?1 AND c.note_id=?3 AND c.x=?4 AND c.y=?5 AND n.trash_root IS NULL),
                        EXISTS(SELECT 1 FROM board_cards WHERE board_id=?2 AND note_id=?3),
                        EXISTS(SELECT 1 FROM occupied o CROSS JOIN board_card_geometry c
                               WHERE c.board_id=?1 AND c.note_id=?3
                                 AND o.x<?6+c.width AND o.x+o.width>?6
                                 AND o.y<?7+c.height AND o.y+o.height>?7)",
            )?;
            for position in expected {
                let BoardItem::Card(note) = position.item else {
                    return Err("這項操作支援筆記、圖片與 PDF 摘錄卡。請取消選取文字方塊和子白板。".into());
                };
                if !selected.insert(note) {
                    return Err("卡片選取重複，請重新選取。".into());
                }
                let x = position.x.checked_add(dx).filter(|x| (0..=10000).contains(x));
                let y = position.y.checked_add(dy).filter(|y| (0..=50000).contains(y));
                let (Some(x), Some(y)) = (x, y) else {
                    return Err("這項位移會超出目的白板的範圍，未搬移任何卡片。".into());
                };
                let (active, duplicate, collision): (bool, bool, bool) = check.query_row(
                    params![source, destination, note, position.x, position.y, x, y],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                if !active {
                    return Err("來源卡片或位置已變更，未搬移任何卡片。請重新選取。".into());
                }
                if duplicate {
                    return Err("目的白板已有選取的卡片，未搬移任何卡片。".into());
                }
                if collision {
                    return Err("目的位置有物件相疊，未搬移任何卡片。請改用其他位移。".into());
                }
                moved.push((note, x, y));
            }
        }

        let mut edges = Vec::new();
        {
            let mut query = tx.prepare("SELECT from_note,to_note FROM board_edges WHERE board_id=?1")?;
            for row in query.query_map([source], |row| Ok((row.get(0)?, row.get(1)?)))? {
                let (from, to): (i64, i64) = row?;
                let endpoints = (selected.contains(&from), selected.contains(&to));
                if endpoints.0 != endpoints.1 {
                    return Err("選取卡片仍連到原板的其他卡片，請一起選取連線兩端再搬移。".into());
                }
                if endpoints.0 {
                    edges.push((from, to));
                }
            }
        }
        let groups = transferable_groups(&tx, source, destination, &selected)?;
        for (note, x, y) in moved {
            tx.execute("INSERT INTO board_cards(board_id,note_id,x,y,image_width,image_height)
                SELECT ?1,note_id,?3,?4,image_width,image_height FROM board_cards
                WHERE board_id=?5 AND note_id=?2",
                params![destination, note, x, y, source])?;
        }
        for (from, to) in edges {
            tx.execute("INSERT INTO board_edges(board_id,from_note,to_note) VALUES(?1,?2,?3)",
                params![destination, from, to])?;
        }
        for (group, members) in groups {
            move_whole_board_group(&tx, source, destination, group, &members)?;
        }
        for note in selected {
            if tx.execute("DELETE FROM board_cards WHERE board_id=?1 AND note_id=?2",
                params![source, note])? != 1
            {
                return Err("來源位置已變更，未搬移任何卡片。".into());
            }
        }
        tx.commit()?;
        Ok(expected.len())
    }

    pub fn move_card_on(&self, board: i64, note: i64, x: i32, y: i32) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE board_cards SET x=?3,y=?4 WHERE board_id=?1 AND note_id=?2
             AND EXISTS(SELECT 1 FROM notes WHERE id=?2 AND trash_root IS NULL)",
            params![board, note, x.clamp(0, 10000), y.clamp(0, 50000)],
        )?;
        if changed == 0 {
            return Err("卡片已移出白板，或筆記已移到垃圾桶。".into());
        }
        Ok(())
    }

    pub fn remove_card_on(&self, board: i64, note: i64) -> Result<()> {
        let changed = self.connection.execute(
            "DELETE FROM board_cards WHERE board_id=?1 AND note_id=?2
             AND EXISTS(SELECT 1 FROM notes WHERE id=?2 AND trash_root IS NULL)",
            params![board, note],
        )?;
        if changed == 0 {
            return Err("卡片已移出白板，或筆記已移到垃圾桶。".into());
        }
        Ok(())
    }

    pub fn edges_on(&self, board: i64) -> Result<Vec<BoardEdge>> {
        let mut query = self.connection.prepare(
            "SELECT e.from_note,e.to_note FROM board_edges e
             JOIN notes source ON source.id=e.from_note
             JOIN notes target ON target.id=e.to_note
             WHERE e.board_id=?1 AND source.trash_root IS NULL AND target.trash_root IS NULL
             ORDER BY e.from_note,e.to_note",
        )?;
        let edges = query.query_map([board], |row| {
            Ok(BoardEdge { from: row.get(0)?, to: row.get(1)? })
        })?;
        Ok(edges.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_edge_on(&self, board: i64, from: i64, to: i64) -> Result<()> {
        if from == to {
            return Err("請選擇兩張不同的卡片。".into());
        }
        let changed = self.connection.execute(
            "INSERT INTO board_edges(board_id,from_note,to_note)
             SELECT ?1,?2,?3
             WHERE 2=(
                SELECT count(*) FROM board_cards c JOIN notes n ON n.id=c.note_id
                WHERE c.board_id=?1 AND c.note_id IN (?2,?3) AND n.trash_root IS NULL
             )
             ON CONFLICT(board_id,from_note,to_note) DO UPDATE SET to_note=excluded.to_note",
            params![board, from.min(to), from.max(to)],
        )?;
        if changed == 0 {
            return Err("請確認兩張卡片都在目前白板上。".into());
        }
        Ok(())
    }

    pub fn remove_edge_on(&self, board: i64, from: i64, to: i64) -> Result<()> {
        self.connection.execute(
            "DELETE FROM board_edges WHERE board_id=?1 AND from_note=?2 AND to_note=?3",
            params![board, from.min(to), from.max(to)],
        )?;
        Ok(())
    }
}

fn transferable_groups(
    tx: &Transaction<'_>, source: i64, destination: i64, selected: &HashSet<i64>,
) -> Result<BTreeMap<i64, Vec<i64>>> {
    let mut query = tx.prepare("SELECT group_id,note_id FROM board_group_cards WHERE board_id=?1")?;
    let mut groups = BTreeMap::<i64, Vec<i64>>::new();
    for row in query.query_map([source], |row| Ok((row.get(0)?, row.get(1)?)))? {
        let (group, card) = row?;
        groups.entry(group).or_default().push(card);
    }
    groups.retain(|_, members| members.iter().any(|note| selected.contains(note)));
    if groups.is_empty() {
        return Ok(groups);
    }
    if groups.values().any(|members| !members.iter().all(|note| selected.contains(note))) {
        return Err("選取只包含分區的一部分。請先展開分區並選取全部成員，再進行搬移。".into());
    }
    let count: i64 = tx.query_row("SELECT count(*) FROM board_groups WHERE board_id=?1", [destination], |row| row.get(0))?;
    if count + groups.len() as i64 > 64 {
        return Err("目的白板的分區空間不足，未搬移任何卡片。".into());
    }
    let mut conflicts = tx.prepare(
        "SELECT EXISTS(SELECT 1 FROM board_groups s JOIN board_groups d ON d.title=s.title
                       WHERE s.board_id=?1 AND d.board_id=?2 AND s.id=?3)",
    )?;
    for group in groups.keys() {
        if conflicts.query_row(params![source, destination, group], |row| row.get::<_, bool>(0))? {
            return Err("目的白板已有同名分區，未搬移任何卡片。".into());
        }
    }
    Ok(groups)
}

fn move_whole_board_group(
    tx: &Transaction<'_>, source: i64, destination: i64, group: i64, members: &[i64],
) -> Result<()> {
    let pin: Option<i64> = tx.query_row(
        "DELETE FROM board_group_sources WHERE board_id=?1 AND group_id=?2 RETURNING annotation_id",
        params![source, group], |row| row.get(0),
    ).optional()?;
    let removed = tx.execute(
        "DELETE FROM board_group_cards WHERE board_id=?1 AND group_id=?2", params![source, group],
    )?;
    if removed != members.len() || tx.execute(
        "UPDATE board_groups SET board_id=?3 WHERE board_id=?1 AND id=?2", params![source, group, destination],
    )? != 1 {
        return Err("來源分區已變更，未搬移任何卡片。".into());
    }
    for note in members {
        tx.execute("INSERT INTO board_group_cards(board_id,note_id,group_id) VALUES(?1,?2,?3)",
            params![destination, note, group])?;
    }
    if let Some(annotation) = pin {
        tx.execute("INSERT INTO board_group_sources(board_id,group_id,annotation_id) VALUES(?1,?2,?3)",
            params![destination, group, annotation])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::path::Path;

    fn snapshot(db: &Notebook, board: i64) -> Result<Vec<BoardPosition>> {
        Ok(db.cards_on(board)?.into_iter().map(|card| BoardPosition {
            item: BoardItem::Card(card.id), x: card.x, y: card.y, version: None,
        }).collect())
    }

    #[test]
    fn cross_board_move_preserves_layout_reference_identity_group_pin_and_internal_edges() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("workspace.sqlite");
        let backup = dir.join("backup.sqlite");
        let pdf = dir.join("原稿.pdf");
        std::fs::write(&pdf, include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf"))?;
        let mut db = Notebook::open(&path, false)?;
        let source = db.create_board("來源")?;
        let destination = db.create_board("目的")?;
        let third = db.create_board("既有參照")?;
        let a = db.add(None, "原卡", "仍是同一份內容")?;
        let paper = db.add(None, "PDF 來源卡", "")?;
        let attachment = db.import_attachment(paper, &pdf)?;
        let region = super::super::PdfRegion { x: 0.12, y: 0.21, width: 0.33, height: 0.11 };
        let annotation = db.add_pdf_annotation_with_region(attachment, 2, "來源選區", "", region)?;
        let excerpt = db.create_pdf_excerpt_card(source, annotation)?;
        db.add_card_on(source, a)?;
        db.add_card_on(source, paper)?;
        db.move_card_on(source, a, 16, 16)?;
        db.move_card_on(source, paper, 276, 16)?;
        db.move_card_on(source, excerpt, 16, 151)?;
        db.add_card_on(third, a)?;
        let other_placement = snapshot(&db, third)?;
        let grouped = db.create_board_group(source, "原分區")?;
        for note in [a, paper, excerpt] {
            db.set_board_group(source, note, None, Some(grouped))?;
        }
        db.set_board_group_source(source, grouped, None, Some(annotation))?;
        db.add_edge_on(source, a, excerpt)?;
        let existing = db.add(None, "目的地原有卡片", "")?;
        db.add_card_on(destination, existing)?;
        db.move_card_on(destination, existing, 970, 800)?;
        let existing_group = db.create_board_group(destination, "保留目的分區")?;
        db.set_board_group(destination, existing, None, Some(existing_group))?;
        let original = snapshot(&db, source)?;
        assert_eq!(db.move_cards_between_boards(source, destination, &original, 0, 135)?, 3);
        assert!(snapshot(&db, source)?.is_empty());
        assert!(db.groups_on(source)?.is_empty());
        assert!(db.edges_on(source)?.is_empty());
        db.backup(&backup)?;
        drop(db);

        let reopened = Notebook::open(&backup, false)?;
        let moved = snapshot(&reopened, destination)?;
        assert_eq!(moved.iter().map(|item| (item.item, item.x, item.y)).collect::<Vec<_>>(), [
            (BoardItem::Card(a), 16, 151), (BoardItem::Card(paper), 276, 151),
            (BoardItem::Card(excerpt), 16, 286), (BoardItem::Card(existing), 970, 800),
        ]);
        assert_eq!(snapshot(&reopened, third)?, other_placement);
        let groups = reopened.groups_on(destination)?;
        assert_eq!((groups[0].id, groups[0].title.as_str(), groups[0].members.as_slice()), (grouped, "原分區", [a, paper, excerpt].as_slice()));
        assert_eq!((groups[1].id, groups[1].members.as_slice()), (existing_group, [existing].as_slice()));
        assert_eq!(reopened.edges_on(destination)?, [BoardEdge { from: a, to: excerpt }]);
        assert_eq!(reopened.note(a)?.unwrap().body, "仍是同一份內容");
        let hit = reopened.live_board_group_source(destination, grouped, annotation)?.unwrap();
        assert_eq!((hit.id, hit.attachment_id, hit.page), (annotation, attachment, 2));
        assert_eq!(reopened.pdf_excerpt_source(excerpt)?.unwrap().id, annotation);
        assert_eq!(reopened.pdf_annotation_focus_regions(attachment, 2, annotation)?, [region]);
        assert_eq!(reopened.connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?, "ok");
        assert!(!reopened.connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)", [], |row| row.get::<_, bool>(0))?);
        drop(reopened);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn cross_board_move_refuses_data_loss_and_rolls_back_a_late_failure() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("workspace.sqlite");
        let mut db = Notebook::open(&path, false)?;
        let source = db.create_board("來源")?;
        let target = db.create_board("目的")?;
        let a = db.add(None, "原卡甲", "")?;
        let b = db.add(None, "原卡乙", "")?;
        db.add_card_on(source, a)?;
        db.add_card_on(source, b)?;
        db.add_edge_on(source, a, b)?;
        let group = db.create_board_group(source, "完整分區")?;
        db.set_board_group(source, a, None, Some(group))?;
        db.set_board_group(source, b, None, Some(group))?;
        let expected = snapshot(&db, source)?;
        assert!(db.move_cards_between_boards(source, target, &expected[..1], 0, 0).is_err());
        db.remove_edge_on(source, a, b)?;
        assert!(db.move_cards_between_boards(source, target, &expected[..1], 0, 0).is_err());
        db.add_edge_on(source, a, b)?;
        let blocker = db.add_board_text(target, "目的板的文字方塊")?;
        assert!(db.move_cards_between_boards(source, target, &expected, 0, 0).is_err());
        assert!(db.move_cards_between_boards(source, target, &expected, i32::MAX, 135).is_err());
        db.add_card_on(target, a)?;
        db.move_card_on(target, a, 1000, 2000)?;
        assert!(db.move_cards_between_boards(source, target, &expected, 0, 135).is_err());
        db.remove_card_on(target, a)?;
        let name_conflict = db.create_board_group(target, "完整分區")?;
        assert!(db.move_cards_between_boards(source, target, &expected, 0, 135).is_err());
        db.connection.execute("DELETE FROM board_groups WHERE board_id=?1 AND id=?2", params![target, name_conflict])?;
        let other = Notebook::open(&path, false)?;
        other.move_card_on(source, b, 300, 16)?;
        assert!(db.move_cards_between_boards(source, target, &expected, 0, 135).is_err());
        let current = snapshot(&db, source)?;
        other.trash(b)?;
        assert!(db.move_cards_between_boards(source, target, &current, 0, 135).is_err());
        db.restore(b)?;
        drop(other);
        db.connection.execute_batch("CREATE TEMP TRIGGER reject_move_source BEFORE DELETE ON board_cards
            BEGIN SELECT RAISE(ABORT,'synthetic move failure'); END;")?;
        assert!(db.move_cards_between_boards(source, target, &current, 0, 135).is_err());
        db.connection.execute_batch("DROP TRIGGER reject_move_source")?;
        assert_eq!(snapshot(&db, source)?, current);
        assert!(snapshot(&db, target)?.is_empty());
        assert_eq!(db.edges_on(source)?, [BoardEdge { from: a, to: b }]);
        assert!(db.edges_on(target)?.is_empty());
        assert_eq!(db.groups_on(source)?[0].members, [a, b]);
        assert!(db.groups_on(target)?.is_empty());
        assert_eq!(db.objects_on(target)?[0].id, blocker);
        assert_eq!(db.move_cards_between_boards(source, target, &current, 0, 135)?, 2);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn reuses_exact_note_and_pdf_excerpt_ids_with_independent_placements_after_reopen() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("workspace.sqlite");
        let backup = dir.join("backup.sqlite");
        let pdf = dir.join("原稿.pdf");
        std::fs::write(&pdf, include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf"))?;
        let mut db = Notebook::open(&path, false)?;
        let original = db.create_board("主題白板")?;
        let target = db.create_board("年代白板")?;
        let a = db.add(None, "原本的筆記", "同一份內容")?;
        let paper = db.add(None, "PDF 來源", "")?;
        let attachment = db.import_attachment(paper, &pdf)?;
        let region = super::super::PdfRegion { x: 0.12, y: 0.21, width: 0.33, height: 0.11 };
        let annotation = db.add_pdf_annotation_with_region(attachment, 2, "來源選區", "", region)?;
        let excerpt = db.create_pdf_excerpt_card(original, annotation)?;
        db.add_card_on(original, a)?;
        db.add_card_on(target, a)?;
        db.move_card_on(target, a, 610, 850)?;
        db.add_board_text(target, "保留這張文字方塊")?;
        db.add_edge_on(original, a, excerpt)?;
        let baseline = snapshot(&db, original)?;

        assert_eq!(db.reuse_board_cards(original, target, &baseline)?, 1);
        assert_eq!(db.reuse_board_cards(original, target, &baseline)?, 0);
        assert_eq!(snapshot(&db, original)?, baseline);
        let destination = snapshot(&db, target)?;
        assert_eq!((destination[0].x, destination[0].y), (610, 850));
        let blocker = &db.objects_on(target)?[0];
        assert!((destination[1].x - blocker.x).abs() >= 232 || (destination[1].y - blocker.y).abs() >= 114);
        assert!(db.edges_on(target)?.is_empty());
        assert_eq!(db.edges_on(original)?, [BoardEdge { from: a, to: excerpt }]);
        let saved = db.note(a)?.unwrap();
        db.save(a, saved.version, "同一個標題", "兩張板共用更新")?;
        db.backup(&backup)?;
        drop(db);

        let reopened = Notebook::open(&backup, false)?;
        assert_eq!(snapshot(&reopened, original)?, baseline);
        assert_eq!(snapshot(&reopened, target)?, destination);
        assert_eq!(reopened.cards_on(original)?[0].excerpt, "兩張板共用更新");
        assert_eq!(reopened.cards_on(target)?[0].excerpt, "兩張板共用更新");
        assert_eq!(reopened.pdf_excerpt_cards_on_board(target)?[0].0, excerpt);
        let hit = reopened.pdf_excerpt_source(excerpt)?.unwrap();
        assert_eq!((hit.note_id, hit.attachment_id, hit.id, hit.page), (paper, attachment, annotation, 2));
        assert_eq!(reopened.pdf_annotation_focus_regions(attachment, 2, annotation)?, [region]);
        reopened.remove_card_on(target, a)?;
        assert_eq!(snapshot(&reopened, original)?, baseline);
        assert!(reopened.note(a)?.is_some());
        drop(reopened);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn batch_reuse_rolls_back_for_mixed_stale_or_over_capacity_selections() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("workspace.sqlite");
        let mut db = Notebook::open(&path, false)?;
        let source = db.create_board("來源")?;
        let target = db.create_board("目的")?;
        let a = db.add(None, "先處理的原卡片", "")?;
        let b = db.add(None, "後處理的原卡片", "")?;
        db.add_card_on(source, a)?;
        db.add_card_on(source, b)?;
        let expected = snapshot(&db, source)?;
        assert!(db.reuse_board_cards(source, source, &expected).is_err());
        assert!(db.reuse_board_cards(source, i64::MAX, &expected).is_err());
        assert!(db.reuse_board_cards(source, target, &[expected[0], expected[0]]).is_err());
        let object = db.add_board_text(source, "不能誤放的文字")?;
        let mixed = [expected[0], BoardPosition { item: BoardItem::Object(object), ..expected[1] }];
        assert!(db.reuse_board_cards(source, target, &mixed).is_err());
        assert!(snapshot(&db, target)?.is_empty());

        let other = Notebook::open(&path, false)?;
        other.move_card_on(source, b, 52, 234)?;
        assert!(db.reuse_board_cards(source, target, &expected).is_err());
        assert!(snapshot(&db, target)?.is_empty());
        let current = snapshot(&db, source)?;
        other.trash(b)?;
        assert!(db.reuse_board_cards(source, target, &current).is_err());
        assert!(snapshot(&db, target)?.is_empty());
        db.restore(b)?;
        drop(other);

        let hidden = db.add(None, "目的白板隱藏的卡片", "")?;
        db.add_card_on(target, hidden)?;
        for _ in 0..MAX_CARDS - 2 {
            let card = db.add(None, "保留既有位置", "")?;
            db.add_card_on(target, card)?;
        }
        db.trash(hidden)?;
        let before = snapshot(&db, target)?;
        assert_eq!(before.len() as i64, MAX_CARDS - 2);
        assert!(db.reuse_board_cards(source, target, &current).is_err());
        assert_eq!(snapshot(&db, target)?, before);
        assert_eq!(snapshot(&db, source)?, current);
        assert_eq!(db.reuse_board_cards(source, target, &current[..1])?, 1);
        assert_eq!(db.reuse_board_cards(source, target, &current[..1])?, 0);
        assert!(db.reuse_board_cards(source, target, &current).is_err());
        assert_eq!(snapshot(&db, target)?.len(), before.len() + 1);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn migrates_single_board_positions_and_note_ids() -> Result<()> {
        let mut connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.execute_batch(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY,title TEXT,body TEXT,trash_root INTEGER);
             CREATE TABLE board_cards(note_id INTEGER PRIMARY KEY REFERENCES notes(id),x INTEGER,y INTEGER);
             INSERT INTO notes VALUES(7,'保留標題','保留本文',NULL),(19,'垃圾桶裡','',19);
             INSERT INTO board_cards VALUES(7,123,456),(19,13,29);",
        )?;
        let tx = connection.transaction()?;
        tx.execute_batch(SCHEMA)?;
        tx.execute_batch(super::super::attachments::SCHEMA)?;
        tx.execute_batch(super::super::board_images::SCHEMA)?;
        tx.commit()?;
        let db = Notebook { connection };
        let boards = db.list_boards()?;
        assert_eq!(boards.len(), 1);
        assert_eq!(boards[0].id, DEFAULT_BOARD_ID);
        let cards = db.cards_on(DEFAULT_BOARD_ID)?;
        assert_eq!(cards.len(), 1);
        assert_eq!((cards[0].id, cards[0].x, cards[0].y), (7, 123, 456));
        assert_eq!(cards[0].excerpt, "保留本文");
        db.connection.execute("UPDATE notes SET trash_root=NULL WHERE id=19", [])?;
        let cards = db.cards_on(DEFAULT_BOARD_ID)?;
        assert_eq!((cards[1].id, cards[1].x, cards[1].y), (19, 13, 29));
        Ok(())
    }

    #[test]
    fn positions_connections_and_trash_are_scoped_to_the_board() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let other = db.create_board(" 研究 ")?;
        assert!(db.create_board("研究").is_err());
        let a = db.add(None, "筆記甲", "")?;
        let b = db.add(None, "筆記乙", "")?;
        db.add_card_on(DEFAULT_BOARD_ID, a)?;
        db.add_card_on(other, a)?;
        assert!(db.add_edge_on(other, a, b).is_err());
        db.add_card_on(other, b)?;
        db.move_card_on(DEFAULT_BOARD_ID, a, 34, 56)?;
        db.move_card_on(other, a, 654, 987)?;
        db.add_edge_on(other, b, a)?;
        db.add_edge_on(other, a, b)?;
        assert!(db.edges_on(DEFAULT_BOARD_ID)?.is_empty());
        assert_eq!(db.edges_on(other)?, [BoardEdge { from: a, to: b }]);
        db.trash(a)?;
        assert!(db.cards_on(DEFAULT_BOARD_ID)?.is_empty());
        assert!(db.edges_on(other)?.is_empty());
        db.restore(a)?;
        let primary = db.cards_on(DEFAULT_BOARD_ID)?;
        let alternate = db.cards_on(other)?;
        assert_eq!((primary[0].x, primary[0].y), (34, 56));
        assert_eq!((alternate[0].x, alternate[0].y), (654, 987));
        assert_eq!(db.edges_on(other)?, [BoardEdge { from: a, to: b }]);
        db.remove_edge_on(other, b, a)?;
        assert!(db.edges_on(other)?.is_empty());
        assert_eq!(db.cards_on(other)?.len(), 2);
        db.add_edge_on(other, a, b)?;
        db.remove_card_on(other, a)?;
        assert!(db.edges_on(other)?.is_empty());
        assert_eq!(db.cards_on(DEFAULT_BOARD_ID)?.len(), 1);
        Ok(())
    }

    #[test]
    fn second_connection_cannot_report_success_for_stale_placements() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-board-race-{}", std::process::id()));
        std::fs::create_dir(&dir)?;
        let path = dir.join("notes.sqlite3");
        {
            let mut first = Notebook::open(&path, false)?;
            let mut second = Notebook::open(&path, false)?;
            let note = first.add(None, "其他視窗的筆記", "")?;
            let board = DEFAULT_BOARD_ID;
            assert!(second.add_card_on(board, note + 1).is_err());
            assert!(second.add_card_on(board + 1, note).is_err());
            first.add_card_on(board, note)?;
            first.move_card_on(board, note, 35, 71)?;
            second.add_card_on(board, note)?;
            assert_eq!((first.cards_on(board)?[0].x, first.cards_on(board)?[0].y), (35, 71));
            first.trash(note)?;
            assert!(second.add_card_on(board, note).is_err());
            assert!(second.move_card_on(board, note, 5, 9).is_err());
            assert!(second.remove_card_on(board, note).is_err());
            first.restore(note)?;
            let card = &first.cards_on(board)?[0];
            assert_eq!((card.x, card.y), (35, 71));
            first.remove_card_on(board, note)?;
            assert!(second.move_card_on(board, note, 5, 9).is_err());
            assert!(second.remove_card_on(board, note).is_err());
            assert!(second.cards_on(board)?.is_empty());
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn card_limit_is_independent_and_includes_hidden_cards() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let alternate = db.create_board("其他")?;
        let first = db.add(None, "先建立", "")?;
        db.add_card_on(DEFAULT_BOARD_ID, first)?;
        for _ in 1..MAX_CARDS {
            let id = db.add(None, "卡片", "")?;
            db.add_card_on(DEFAULT_BOARD_ID, id)?;
        }
        db.add_card_on(DEFAULT_BOARD_ID, first)?;
        db.trash(first)?;
        let extra = db.add(None, "超出容量", "")?;
        assert!(db.add_card_on(DEFAULT_BOARD_ID, extra).is_err());
        db.add_card_on(alternate, extra)?;
        assert_eq!(db.cards_on(alternate)?[0].id, extra);
        assert!(db.remove_card_on(DEFAULT_BOARD_ID, first).is_err());
        db.restore(first)?;
        db.remove_card_on(DEFAULT_BOARD_ID, first)?;
        db.add_card_on(DEFAULT_BOARD_ID, extra)?;
        let cards = db.cards_on(DEFAULT_BOARD_ID)?;
        assert_eq!(cards.len() as i64, MAX_CARDS);
        assert_eq!((cards.last().unwrap().x, cards.last().unwrap().y), (16, 16));
        Ok(())
    }
}
