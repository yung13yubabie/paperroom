//! Board-local topic trees. Their anchors participate in the ordinary canvas layout.
use super::board_objects::position;
use super::{Notebook, Result};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

pub(super) const SCHEMA: &str = "
CREATE UNIQUE INDEX IF NOT EXISTS board_objects_board_mind_anchor ON board_objects(board_id,id);
CREATE TABLE board_mind_nodes(
 id INTEGER PRIMARY KEY,
 board_id INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
 parent_id INTEGER,
 title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 160),
 UNIQUE(board_id,id),
 FOREIGN KEY(board_id,id) REFERENCES board_objects(board_id,id) ON DELETE CASCADE,
 FOREIGN KEY(board_id,parent_id) REFERENCES board_mind_nodes(board_id,id)
    DEFERRABLE INITIALLY DEFERRED,
 CHECK(parent_id IS NULL OR parent_id<>id)
);
CREATE INDEX board_mind_nodes_parent ON board_mind_nodes(board_id,parent_id);";

pub(crate) const TITLE_LIMIT: usize = 160;
const MAX_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MindNode {
    pub id: i64,
    pub parent: Option<i64>,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub version: i64,
}

fn title(value: &str) -> Result<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > TITLE_LIMIT
        || trimmed.chars().any(char::is_control)
    {
        return Err("心智圖主題需有 1 至 160 字，且不能包含換行或控制字元。".into());
    }
    Ok(trimmed)
}

fn canvas_text(title: &str) -> String {
    let mut escaped = String::with_capacity(title.len());
    // JSON Canvas text is Markdown. Topic titles are always literal text.
    for ch in title.chars() {
        if ch.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn nodes_on(db: &Connection, board: i64) -> Result<Vec<MindNode>> {
    let mut stmt = db.prepare(
        "SELECT m.id,m.parent_id,m.title,o.x,o.y,o.version
         FROM board_mind_nodes m JOIN board_objects o ON o.id=m.id AND o.board_id=m.board_id
         WHERE m.board_id=?1 ORDER BY m.id",
    )?;
    Ok(stmt.query_map([board], |row| Ok(MindNode {
        id: row.get(0)?,
        parent: row.get(1)?,
        title: row.get(2)?,
        x: row.get(3)?,
        y: row.get(4)?,
        version: row.get(5)?,
    }))?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn current<'a>(nodes: &'a [MindNode], expected: &MindNode) -> Result<&'a MindNode> {
    nodes.iter().find(|node| *node == expected)
        .ok_or_else(|| "心智圖節點已變更，請重新開啟操作。".into())
}

fn depth(nodes: &[MindNode], mut id: i64, moving: Option<i64>) -> Result<usize> {
    for depth in 1..=MAX_DEPTH {
        if moving == Some(id) {
            return Err("不能將節點移入自己或自己的子節點。".into());
        }
        let node = nodes.iter().find(|node| node.id == id).ok_or("找不到心智圖父節點。")?;
        match node.parent {
            Some(parent) => id = parent,
            None => return Ok(depth),
        }
    }
    Err("心智圖階層最多 16 層。".into())
}

fn branch(nodes: &[MindNode], id: i64) -> Result<Vec<(i64, usize)>> {
    let mut result = vec![(id, 1)];
    let mut index = 0;
    while let Some(&(parent, level)) = result.get(index) {
        for child in nodes.iter().filter(|node| node.parent == Some(parent)) {
            if level == MAX_DEPTH || result.iter().any(|(id, _)| *id == child.id) {
                return Err("心智圖階層包含循環或超過 16 層。".into());
            }
            result.push((child.id, level + 1));
        }
        index += 1;
    }
    Ok(result)
}

fn touch_anchor(tx: &Transaction<'_>, board: i64, node: &MindNode) -> Result<()> {
    if tx.execute(
        "UPDATE board_objects SET version=version+1 WHERE board_id=?1 AND id=?2 AND version=?3",
        params![board, node.id, node.version],
    )? != 1 {
        return Err("心智圖節點已變更，請重新開啟操作。".into());
    }
    Ok(())
}

impl Notebook {
    pub(crate) fn mind_nodes_on(&self, board: i64) -> Result<Vec<MindNode>> {
        nodes_on(&self.connection, board)
    }

    pub(crate) fn add_mind_node(&self, board: i64, parent: Option<&MindNode>, value: &str) -> Result<i64> {
        let value = title(value)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let preferred = if let Some(parent) = parent {
            let nodes = nodes_on(&tx, board)?;
            let parent = current(&nodes, parent)?;
            if depth(&nodes, parent.id, None)? >= MAX_DEPTH {
                return Err("心智圖階層最多 16 層。".into());
            }
            let x = parent.x + 260;
            (x <= 276).then_some((x, parent.y))
        } else {
            None
        };
        let (x, y) = position(&tx, board, preferred)?;
        tx.execute("INSERT INTO board_objects(board_id,x,y) VALUES(?1,?2,?3)", params![board, x, y])?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO board_mind_nodes(id,board_id,parent_id,title) VALUES(?1,?2,?3,?4)",
            params![id, board, parent.map(|node| node.id), value],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub(crate) fn edit_mind_node(&self, board: i64, expected: &MindNode, value: &str) -> Result<()> {
        let value = title(value)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let nodes = nodes_on(&tx, board)?;
        let node = current(&nodes, expected)?;
        if node.title != value {
            touch_anchor(&tx, board, node)?;
            tx.execute("UPDATE board_mind_nodes SET title=?2 WHERE id=?1", params![node.id, value])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn reparent_mind_node(
        &self, board: i64, expected: &MindNode, parent: Option<&MindNode>,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let nodes = nodes_on(&tx, board)?;
        let node = current(&nodes, expected)?;
        let parent = parent.map(|expected| current(&nodes, expected)).transpose()?;
        if node.parent != parent.map(|parent| parent.id) {
            if let Some(parent) = parent {
                let parent_depth = depth(&nodes, parent.id, Some(node.id))?;
                let branch_height = branch(&nodes, node.id)?.last().map_or(1, |(_, level)| *level);
                if parent_depth + branch_height > MAX_DEPTH {
                    return Err("移動後的心智圖階層超過 16 層。".into());
                }
            }
            touch_anchor(&tx, board, node)?;
            tx.execute("UPDATE board_mind_nodes SET parent_id=?2 WHERE id=?1",
                params![node.id, parent.map(|parent| parent.id)])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn remove_mind_leaf(&self, board: i64, expected: &MindNode) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let nodes = nodes_on(&tx, board)?;
        let node = current(&nodes, expected)?;
        if nodes.iter().any(|child| child.parent == Some(node.id)) {
            return Err("先移動或逐一刪除子節點，才能刪除這個節點。".into());
        }
        if tx.execute(
            "DELETE FROM board_objects WHERE board_id=?1 AND id=?2 AND version=?3",
            params![board, node.id, node.version],
        )? != 1 {
            return Err("心智圖節點已變更，請重新開啟操作。".into());
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn mind_branch(&self, board: i64, expected: &MindNode) -> Result<Vec<i64>> {
        let nodes = self.mind_nodes_on(board)?;
        let node = current(&nodes, expected)?;
        Ok(branch(&nodes, node.id)?.into_iter().map(|(id, _)| id).collect())
    }

    pub(crate) fn export_mind_branch(&self, board: i64, expected: &MindNode, path: &Path) -> Result<usize> {
        let nodes = self.mind_nodes_on(board)?;
        let node = current(&nodes, expected)?;
        let branch = branch(&nodes, node.id)?;
        let indexes: HashMap<_, _> = branch.iter().enumerate()
            .map(|(index, entry)| (entry.0, index + 1)).collect();
        let mut canvas_nodes = Vec::with_capacity(branch.len());
        let mut edges = Vec::with_capacity(branch.len() - 1);
        for (id, _) in branch {
            let node = nodes.iter().find(|node| node.id == id).ok_or("找不到心智圖節點。")?;
            let index = indexes[&id];
            let parent = node.parent.and_then(|parent| indexes.get(&parent).copied());
            canvas_nodes.push(json!({
                "id": format!("topic-{index}"), "type": "text", "text": canvas_text(&node.title),
                "x": node.x, "y": node.y, "width": 232, "height": 114,
                "color": if parent.is_none() { "#1C6870" } else { "#E6F4F5" },
            }));
            if let Some(parent) = parent {
                edges.push(json!({
                    "id": format!("branch-{index}"), "fromNode": format!("topic-{parent}"),
                    "toNode": format!("topic-{index}"), "fromEnd": "none", "toEnd": "arrow",
                }));
            }
        }
        let document = crate::canvas::parse(serde_json::to_string_pretty(&json!({
            "nodes": canvas_nodes, "edges": edges,
        }))?)?;
        crate::canvas::export(path, &document)?;
        Ok(canvas_nodes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::board_images::BoardImageInput;
    use super::super::board_objects::{BoardItem, BoardPosition};

    fn node(db: &Notebook, board: i64, id: i64) -> Result<MindNode> {
        db.mind_nodes_on(board)?.into_iter().find(|node| node.id == id)
            .ok_or_else(|| "synthetic node missing".into())
    }

    #[test]
    fn branch_layout_labels_backup_and_standard_canvas_preserve_relationships() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("working.sqlite3");
        let backup = dir.join("saved.sqlite3");
        let canvas = dir.join("portable.canvas");
        let mut db = Notebook::open(&path, false)?;
        let board = db.create_board("分鏡研究")?;
        let legacy = db.add_board_text(board, "既有文字方塊")?;
        let root = db.add_mind_node(board, None, " 紙間 & 鏡頭 ")?;
        let first = db.add_mind_node(board, Some(&node(&db, board, root)?), "第一分支")?;
        let second = db.add_mind_node(board, Some(&node(&db, board, root)?), "第二分支")?;
        let leaf = db.add_mind_node(board, Some(&node(&db, board, first)?), "鏡頭來源")?;
        let root_before = node(&db, board, root)?;
        assert_eq!(root_before.title, "紙間 & 鏡頭");
        assert_eq!(db.mind_branch(board, &root_before)?, [root, first, second, leaf]);
        assert!(db.remove_mind_leaf(board, &root_before).is_err());
        assert!(db.reparent_mind_node(board, &root_before, Some(&node(&db, board, leaf)?)).is_err());
        assert_eq!(node(&db, board, root)?, root_before);

        let first_before = node(&db, board, first)?;
        let leaf_before = node(&db, board, leaf)?;
        db.reparent_mind_node(board, &leaf_before, Some(&node(&db, board, second)?))?;
        let leaf_after = node(&db, board, leaf)?;
        assert_eq!((leaf_after.x, leaf_after.y, leaf_after.parent),
            (leaf_before.x, leaf_before.y, Some(second)));
        assert_eq!(db.mind_branch(board, &first_before)?, [first]);
        assert!(db.edit_mind_node(board, &leaf_before, "不能覆蓋").is_err());
        db.edit_mind_node(board, &leaf_after, "精確畫面 ![[附件]] <b>")?;
        let leaf_anchor = db.objects_on(board)?.into_iter().find(|object| object.id == leaf).unwrap();
        assert!(db.remove_board_text(board, &leaf_anchor, None).is_err());
        assert!(db.edit_board_text(board, &leaf_anchor, "不能覆蓋").is_err());
        assert!(db.latest_removed_board_text(board)?.is_none());
        assert_eq!(leaf_anchor.text, "");

        let selected: Vec<_> = db.mind_nodes_on(board)?.into_iter().map(|node| BoardPosition {
            item: BoardItem::Object(node.id), x: node.x, y: node.y, version: Some(node.version),
        }).collect();
        db.move_board_selection(board, &selected, 510, 30)?;
        assert!(db.move_board_selection(board, &selected, 1, 0).is_err());
        for (entry, node) in selected.iter().zip(db.mind_nodes_on(board)?) {
            assert_eq!((node.x, node.y), (entry.x + 510, entry.y + 30));
        }
        let second_latest = node(&db, board, second)?;
        assert_eq!(db.export_mind_branch(board, &second_latest, &canvas)?, 2);
        assert!(db.export_mind_branch(board, &second_latest, &canvas).is_err());
        let exported = crate::canvas::read(&canvas)?;
        assert_eq!(exported.json()["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(exported.json()["nodes"][0]["id"], "topic-1");
        assert_eq!(exported.json()["nodes"][0]["x"], second_latest.x);
        assert_eq!(exported.json()["nodes"][1]["text"], r"精確畫面 \!\[\[附件\]\] \<b\>");
        assert_eq!(exported.json()["edges"][0]["fromNode"], "topic-1");
        assert_eq!(exported.json()["edges"][0]["toNode"], "topic-2");
        assert_eq!(exported.json()["edges"][0]["toEnd"], "arrow");
        assert!(!exported.as_str().contains("紙間 & 鏡頭"));

        let expected = db.mind_nodes_on(board)?;
        db.backup(&backup)?;
        drop(db);
        let db = Notebook::open(&backup, false)?;
        assert_eq!(db.mind_nodes_on(board)?, expected);
        assert_eq!(db.objects_on(board)?.into_iter().find(|entry| entry.id == legacy).unwrap().text,
            "既有文字方塊");
        db.remove_mind_leaf(board, &node(&db, board, leaf)?)?;
        assert_eq!(db.mind_branch(board, &node(&db, board, root)?)?, [root, first, second]);
        assert_ne!(db.add_mind_node(board, None, "新的節點")?, leaf);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn caps_cycles_board_boundary_collision_and_failed_writes_leave_data_intact() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let db = Notebook::open(&dir.join("synthetic.sqlite3"), false)?;
        let board = db.create_board("先保留")?;
        let other_board = db.create_board("其他白板")?;
        let root = db.add_mind_node(board, None, "根")?;
        let mut parent = node(&db, board, root)?;
        let other = db.add_mind_node(other_board, None, "異板")?;
        let other = node(&db, other_board, other)?;
        assert!(db.add_mind_node(board, Some(&other), "異板子節點").is_err());
        assert!(db.reparent_mind_node(board, &parent, Some(&other)).is_err());
        assert!(db.add_mind_node(board, None, "不接受\n多行").is_err());
        assert!(db.add_mind_node(board, None, &"界".repeat(TITLE_LIMIT + 1)).is_err());
        assert!(db.connection.execute("INSERT INTO board_mind_nodes(id,board_id,title) VALUES(?1,?2,'X')",
            params![other.id + 1, other_board]).is_err());

        let original = parent.clone();
        for level in 2..=MAX_DEPTH {
            let child = db.add_mind_node(board, Some(&parent), &format!("第 {level} 層"))?;
            parent = node(&db, board, child)?;
        }
        let expected = db.mind_nodes_on(board)?;
        assert!(db.add_mind_node(board, Some(&parent), "不可能第 17 層").is_err());
        let extra = db.add_mind_node(board, None, "獨立根")?;
        assert!(db.reparent_mind_node(board, &original, Some(&node(&db, board, extra)?)).is_err());
        assert!(db.reparent_mind_node(board, &original, Some(&parent)).is_err());
        db.connection.execute_batch("CREATE TEMP TRIGGER reject_topic BEFORE INSERT ON board_mind_nodes
            BEGIN SELECT RAISE(ABORT,'synthetic topic failure'); END;")?;
        assert!(db.add_mind_node(board, None, "不會留下空白錨點").is_err());
        assert_eq!(db.objects_on(board)?.len(), expected.len() + 1);
        assert_eq!(&db.mind_nodes_on(board)?[..expected.len()], expected.as_slice());
        db.connection.execute_batch("DROP TRIGGER reject_topic;")?;

        use image::ImageEncoder;
        let mut pixels = Vec::new();
        image::codecs::png::PngEncoder::new(&mut pixels).write_image(
            &[80, 140, 160, 255], 1, 1, image::ExtendedColorType::Rgba8,
        )?;
        let picture = BoardImageInput::new("大型圖片".to_owned(), pixels)?;
        let image = db.add_board_images(board, &[picture])?[0];
        db.connection.execute(
            "UPDATE board_cards SET x=400,y=16,image_width=768,image_height=220 WHERE board_id=?1 AND note_id=?2",
            params![board, image],
        )?;
        let pos = BoardPosition { item: BoardItem::Object(root), x: original.x, y: original.y, version: Some(original.version) };
        assert!(db.move_board_selection(board, &[pos], 850, 0).is_err());
        assert_eq!(node(&db, board, root)?, original);
        db.connection.execute("DELETE FROM boards WHERE id=?1", [board])?;
        assert!(db.mind_nodes_on(board)?.is_empty());
        assert_eq!(db.mind_nodes_on(other_board)?, [other]);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
