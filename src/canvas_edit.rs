use super::{Notebook, Result};
use crate::canvas::{self, Document};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::HashSet;

pub(super) const UNDO_SCHEMA: &str = "
CREATE TABLE canvas_edit_undo(
    canvas_id INTEGER NOT NULL UNIQUE REFERENCES saved_canvases(id) ON DELETE CASCADE,
    source TEXT NOT NULL
);
CREATE TRIGGER clear_canvas_edit_undo AFTER UPDATE OF source ON saved_canvases
WHEN old.source <> new.source
BEGIN
    DELETE FROM canvas_edit_undo WHERE canvas_id = old.id;
END;";

#[derive(Clone, Copy)]
pub(crate) enum EdgeStyleField { FromSide, ToSide, FromEnd, ToEnd, Color }

impl EdgeStyleField {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::FromSide => "fromSide",
            Self::ToSide => "toSide",
            Self::FromEnd => "fromEnd",
            Self::ToEnd => "toEnd",
            Self::Color => "color",
        }
    }
}

pub(crate) enum CanvasEdit<'a> {
    AddNode { kind: &'a str, content: &'a str, geometry: [i64; 4] },
    Geometry { id: &'a str, geometry: [i64; 4] },
    Translate { ids: &'a HashSet<String>, offset: [i64; 2] },
    Content { id: &'a str, value: &'a str },
    NodeLabel { id: &'a str, label: Option<&'a str> },
    ZOrder { id: &'a str, front: bool },
    DeleteNode { id: &'a str },
    AddEdge { from: &'a str, to: &'a str, label: Option<&'a str> },
    EdgeLabel { id: &'a str, label: Option<&'a str> },
    DeleteEdge { id: &'a str },
    EdgeStyle { id: &'a str, field: EdgeStyleField, value: Option<&'a str> },
}

fn content_key(kind: &str) -> Result<&'static str> {
    match kind {
        "text" => Ok("text"),
        "file" => Ok("file"),
        "link" => Ok("url"),
        "group" => Ok("label"),
        _ => Err("找不到這種 Canvas 節點類型。".into()),
    }
}

fn array<'a>(tree: &'a mut Value, field: &str) -> Result<&'a mut Vec<Value>> {
    let root = tree.as_object_mut().ok_or("Canvas 缺少根物件。")?;
    root.entry(field).or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut().ok_or_else(|| "Canvas 的節點或連線欄位格式錯誤。".into())
}

fn position(items: &[Value], id: &str) -> Result<usize> {
    items.iter().position(|item| item["id"].as_str() == Some(id))
        .ok_or_else(|| "找不到選取的 Canvas 項目，請重新開啟畫布。".into())
}

fn unique_id(tree: &Value, prefix: &str) -> String {
    let ids: HashSet<&str> = ["nodes", "edges"].iter()
        .flat_map(|field| tree[*field].as_array().into_iter().flatten())
        .filter_map(|item| item["id"].as_str()).collect();
    let mut suffix = 1;
    loop {
        let id = format!("paperroom-{prefix}-{suffix}");
        if !ids.contains(id.as_str()) { return id; }
        suffix += 1;
    }
}

fn text(value: &str) -> Result<Value> {
    if value.len() > 8 * 1024 * 1024 {
        return Err("文字超過 Canvas 文件的 8 MiB 容量上限。".into());
    }
    Ok(Value::String(value.to_owned()))
}

fn optional_text(item: &mut Value, field: &str, value: Option<&str>) -> Result<()> {
    let object = item.as_object_mut().ok_or("Canvas 項目格式錯誤。")?;
    match value {
        Some(value) => { object.insert(field.to_owned(), text(value)?); }
        None => { object.remove(field); }
    }
    Ok(())
}

fn set_geometry(node: &mut Value, geometry: [i64; 4]) {
    for (field, value) in ["x", "y", "width", "height"].into_iter().zip(geometry) {
        node[field] = Value::from(value);
    }
}

fn apply(original: &Document, edit: CanvasEdit<'_>) -> Result<Document> {
    let mut tree = original.json().clone();
    match edit {
        CanvasEdit::AddNode { kind, content, geometry } => {
            let field = content_key(kind)?;
            let mut node = json!({"id":unique_id(&tree, "node"), "type":kind});
            node[field] = text(content)?;
            set_geometry(&mut node, geometry);
            array(&mut tree, "nodes")?.push(node);
        }
        CanvasEdit::Geometry { id, geometry } => {
            let nodes = array(&mut tree, "nodes")?;
            let index = position(nodes, id)?;
            set_geometry(&mut nodes[index], geometry);
        }
        CanvasEdit::Translate { ids, offset } => {
            let mut found = 0;
            for node in array(&mut tree, "nodes")? {
                if !node["id"].as_str().is_some_and(|id| ids.contains(id)) { continue; }
                for (field, amount) in ["x", "y"].into_iter().zip(offset) {
                    if amount == 0 { continue; }
                    // Drawing accepts integer notation such as 1.2e2. Every
                    // integer in the editable range is exactly representable.
                    let value = node[field].as_f64()
                        .filter(|origin| origin.abs() <= crate::canvas_projection::MAX_COORDINATE as f64
                            && canvas::integer(node.get(field)))
                        .map(|origin| origin as i64)
                        .and_then(|origin| origin.checked_add(amount))
                        .filter(|value| value.abs_diff(0) <= crate::canvas_projection::MAX_COORDINATE as u64)
                        .ok_or("無法安全移動選取的節點。請檢查座標是否為可編輯範圍內的整數。")?;
                    node[field] = Value::from(value);
                }
                found += 1;
            }
            if found != ids.len() { return Err("有選取的節點已不存在。請重新開啟畫布。".into()); }
        }
        CanvasEdit::Content { id, value } => {
            let nodes = array(&mut tree, "nodes")?;
            let index = position(nodes, id)?;
            let node = &mut nodes[index];
            let key = content_key(node["type"].as_str().ok_or("Canvas 節點缺少類型。")?)?;
            node[key] = text(value)?;
        }
        CanvasEdit::NodeLabel { id, label: value } | CanvasEdit::EdgeLabel { id, label: value } => {
            let field = if matches!(edit, CanvasEdit::NodeLabel { .. }) { "nodes" } else { "edges" };
            let items = array(&mut tree, field)?;
            let index = position(items, id)?;
            optional_text(&mut items[index], "label", value)?;
        }
        CanvasEdit::ZOrder { id, front } => {
            let nodes = array(&mut tree, "nodes")?;
            let index = position(nodes, id)?;
            let node = nodes.remove(index);
            if front { nodes.push(node); } else { nodes.insert(0, node); }
        }
        CanvasEdit::DeleteNode { id } => {
            let nodes = array(&mut tree, "nodes")?;
            let index = position(nodes, id)?;
            nodes.remove(index);
            if let Some(edges) = tree.get_mut("edges").and_then(Value::as_array_mut) {
                edges.retain(|e| e["fromNode"].as_str() != Some(id) && e["toNode"].as_str() != Some(id));
            }
        }
        CanvasEdit::AddEdge { from, to, label: value } => {
            let nodes = tree["nodes"].as_array().ok_or("先在畫布加入節點。")?;
            position(nodes, from)?;
            position(nodes, to)?;
            let mut edge = json!({"id":unique_id(&tree, "edge"), "fromNode":from, "toNode":to});
            optional_text(&mut edge, "label", value)?;
            array(&mut tree, "edges")?.push(edge);
        }
        CanvasEdit::EdgeStyle { id, field, value } => {
            let edges = array(&mut tree, "edges")?;
            let index = position(edges, id)?;
            optional_text(&mut edges[index], field.key(), value)?;
        }
        CanvasEdit::DeleteEdge { id } => {
            let edges = array(&mut tree, "edges")?;
            let index = position(edges, id)?;
            edges.remove(index);
        }
    }
    serialize_edit(original, tree)
}

pub(super) fn serialize_edit(original: &Document, tree: Value) -> Result<Document> {
    // Preserve the exact original formatting on a no-op. A real edit writes
    // compact JSON, retaining extension values and arbitrary-precision numbers.
    if tree == *original.json() {
        return original.reparse(original.as_str().to_owned());
    }
    let mut raw = String::new();
    if original.as_str().starts_with('\u{feff}') { raw.push('\u{feff}'); }
    raw.push_str(&serde_json::to_string(&tree)?);
    original.reparse(raw)
}

impl Notebook {
    pub(crate) fn edit_canvas(
        &self, canvas_id: i64, expected: &Document, edit: CanvasEdit<'_>,
    ) -> Result<Document> {
        let revised = apply(expected, edit)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        replace_source(&tx, canvas_id, expected, &revised)?;
        if revised.as_str() != expected.as_str() {
            tx.execute("INSERT INTO canvas_edit_undo(canvas_id, source) VALUES (?1, ?2)",
                params![canvas_id, expected.as_str()])?;
            // Keep the most recent action per canvas with a workspace-wide byte budget.
            tx.execute(
                "DELETE FROM canvas_edit_undo WHERE rowid IN (
                    SELECT rowid FROM (
                        SELECT rowid, sum(length(CAST(source AS BLOB))) OVER (ORDER BY rowid DESC) AS bytes
                        FROM canvas_edit_undo
                    ) WHERE bytes > ?1
                 )", [super::workspace::MAX_CANVAS_STORAGE_BYTES],
            )?;
        }
        tx.commit()?;
        Ok(revised)
    }

    pub(crate) fn undo_canvas_edit(&self, canvas_id: i64, expected: &Document) -> Result<Option<Document>> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx.query_row(
            "SELECT source FROM canvas_edit_undo WHERE canvas_id=?1", [canvas_id], |row| row.get(0),
        ).optional()?;
        let Some(previous) = previous else { return Ok(None); };
        let restored = expected.reparse(previous)?;
        replace_source(&tx, canvas_id, expected, &restored)?;
        tx.commit()?;
        Ok(Some(restored))
    }
}

fn replace_source(
    tx: &Transaction<'_>, canvas_id: i64, expected: &Document, revised: &Document,
) -> Result<()> {
    let changed = tx.execute(
        "UPDATE saved_canvases SET source=?3
         WHERE id=?1 AND source=?2
         AND length(CAST(?3 AS BLOB)) +
             (SELECT coalesce(sum(length(CAST(source AS BLOB))),0)
              FROM saved_canvases WHERE id<>?1) <= ?4",
        params![canvas_id, expected.as_str(), revised.as_str(), super::workspace::MAX_CANVAS_STORAGE_BYTES],
    )?;
    if changed == 0 {
        return Err("畫布已更改、已移除，或工作區的 Canvas 已達容量上限。請重新開啟畫布。".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use CanvasEdit::*;

    #[test]
    fn group_translation_undo_reopens_and_keeps_links_and_untouched_data() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let db = Notebook::open(&path, false)?;
        let raw = " {\n\"nodes\":[{\"id\":\"a\",\"type\":\"text\",\"x\":1.2e2,\"y\":-10,\"width\":81,\"height\":50,\"text\":\"文字\"},{\"id\":\"b\",\"type\":\"file\",\"x\":5,\"y\":30,\"width\":12,\"height\":20,\"file\":\"資料/筆記.md\",\"subpath\":\"#一\"},{\"id\":\"c\",\"type\":\"group\",\"x\":7,\"y\":11,\"width\":111,\"height\":99}],\"edges\":[{\"id\":\"e\",\"fromNode\":\"a\",\"toNode\":\"b\",\"extra\":true}]}\n";
        let starting = canvas::parse(raw.into())?;
        let id = db.save_canvas("合成畫布", &starting)?;
        let ids = HashSet::from(["a".to_owned(), "b".to_owned()]);
        let bad_ids = HashSet::from(["a".to_owned(), "missing".to_owned()]);
        assert!(db.edit_canvas(id, &starting, Translate { ids: &bad_ids, offset: [2, 3] }).is_err());
        assert!(db.edit_canvas(id, &starting, Translate { ids: &ids, offset: [i64::MAX, 1] }).is_err());
        assert_eq!(db.read_canvas(id)?.as_str(), raw);
        assert!(db.undo_canvas_edit(id, &starting)?.is_none());
        let moved = db.edit_canvas(id, &starting, Translate { ids: &ids, offset: [-7, 9] })?;
        assert_eq!(moved.json()["nodes"][0]["x"], 113);
        assert_eq!(moved.json()["nodes"][1]["x"], -2);
        assert_eq!(moved.json()["nodes"][0]["y"], -1);
        assert_eq!(moved.json()["nodes"][1]["y"], 39);
        for key in ["id", "file", "subpath", "width", "height"] {
            assert_eq!(moved.json()["nodes"][1][key], starting.json()["nodes"][1][key]);
        }
        assert_eq!(moved.json()["nodes"][2], starting.json()["nodes"][2]);
        assert_eq!(moved.json()["edges"], starting.json()["edges"]);
        assert!(db.undo_canvas_edit(id, &starting).is_err());
        drop(db);

        let db = Notebook::open(&path, false)?;
        let reopened = db.read_canvas(id)?;
        assert_eq!(reopened.as_str(), moved.as_str());
        let undone = db.undo_canvas_edit(id, &reopened)?.unwrap();
        assert_eq!(undone.as_str(), raw);
        assert_eq!(db.read_canvas(id)?.as_str(), raw);
        assert!(db.undo_canvas_edit(id, &undone)?.is_none());
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn undo_of_edge_deletion_is_exact_and_other_source_writes_invalidate_undo() -> Result<()> {
        let db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let initial = canvas::parse(r#"{"nodes":[{"id":"a","type":"group","x":0,"y":0,"width":1,"height":1},{"id":"b","type":"group","x":2,"y":2,"width":1,"height":1}],"edges":[{"id":"line","fromNode":"a","toNode":"b","extra":42}]}"#.to_owned())?;
        let id = db.save_canvas("連線", &initial)?;
        let disconnected = db.edit_canvas(id, &initial, DeleteEdge { id: "line" })?;
        assert_eq!(disconnected.edges, 0);
        let no_op = db.edit_canvas(id, &disconnected, ZOrder { id: "b", front: true })?;
        let restored = db.undo_canvas_edit(id, &no_op)?.unwrap();
        assert_eq!(restored.as_str(), initial.as_str());
        let changed = db.edit_canvas(id, &restored, DeleteEdge { id: "line" })?;
        // Workspace operations such as Vault relocation may write the source directly.
        db.connection.execute("UPDATE saved_canvases SET source=?2 WHERE id=?1",
            params![id, format!(" {}", changed.as_str())])?;
        let current = db.read_canvas(id)?;
        assert!(db.undo_canvas_edit(id, &current)?.is_none());
        assert_eq!(db.read_canvas(id)?.as_str(), current.as_str());
        Ok(())
    }

    #[test]
    fn edits_preserve_extensions_precise_numbers_and_native_node_order() -> Result<()> {
        let initial = r##"{"vendor":{"$serde_json::private::Number":"literal","n":987654321987654321987654321987654321},"nodes":[{"id":"old","type":"group","x":1e500,"y":0,"width":12,"height":34,"background":"unknown.png","color":"#ABC"}]}"##;
        let mut doc = canvas::parse(initial.into())?;
        let extensions = doc.json()["vendor"].clone();
        let db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let id = db.save_canvas("合成畫布", &doc)?;
        for (kind, content) in [("text","文字"), ("file","資料/筆記.md"), ("link","https://example.invalid"), ("group","區段")] {
            doc = db.edit_canvas(id, &doc, AddNode { kind, content, geometry: [-2, 3, 70, 80] })?;
        }
        let a = "paperroom-node-1";
        let b = "paperroom-node-2";
        doc = db.edit_canvas(id, &doc, Geometry { id: a, geometry: [1, 2, 3, 4] })?;
        doc = db.edit_canvas(id, &doc, Content { id: a, value: "已編輯" })?;
        doc = db.edit_canvas(id, &doc, NodeLabel { id: b, label: Some("別名") })?;
        doc = db.edit_canvas(id, &doc, ZOrder { id: a, front: false })?;
        assert_eq!(doc.json()["nodes"][0]["text"], "已編輯");
        assert_eq!(doc.json()["nodes"][0]["height"], 4);
        doc = db.edit_canvas(id, &doc, ZOrder { id: a, front: true })?;
        assert_eq!(doc.json()["nodes"][4]["id"], a);
        doc = db.edit_canvas(id, &doc, AddEdge { from: a, to: b, label: None })?;
        doc = db.edit_canvas(id, &doc, AddEdge { from: b, to: "old", label: Some("返回") })?;
        doc = db.edit_canvas(id, &doc, EdgeLabel { id: "paperroom-edge-1", label: Some("連結") })?;
        assert_eq!(doc.json()["edges"][0]["label"], "連結");
        doc = db.edit_canvas(id, &doc, DeleteEdge { id: "paperroom-edge-2" })?;
        doc = db.edit_canvas(id, &doc, AddEdge { from: b, to: a, label: None })?;
        doc = db.edit_canvas(id, &doc, AddEdge { from: a, to: "old", label: Some("保留") })?;
        doc = db.edit_canvas(id, &doc, DeleteNode { id: b })?;
        assert_eq!((doc.nodes, doc.edges), (4, 1));
        assert_eq!(doc.json()["edges"][0]["label"], "保留");
        assert_eq!(doc.json()["vendor"], extensions);
        assert_eq!(doc.json()["nodes"][0]["color"], "#ABC");
        assert_eq!(doc.json()["nodes"][0]["x"].as_number().unwrap().as_str(), "1e+500");
        assert_eq!(db.read_canvas(id)?.as_str(), doc.as_str());
        Ok(())
    }

    #[test]
    fn edge_style_edits_preserve_connections_labels_and_extensions() -> Result<()> {
        use EdgeStyleField::*;
        let db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let original = canvas::parse(r#"{"nodes":[{"id":"a","type":"group","x":0,"y":0,"width":1,"height":1},{"id":"b","type":"group","x":2,"y":2,"width":1,"height":1}],"edges":[{"id":"line","fromNode":"a","toNode":"b","label":"字幕","extra":{"weight":3}}]}"#.to_owned())?;
        let id = db.save_canvas("線條樣式", &original)?;
        let mut doc = original;
        for (field, value) in [(FromSide,"left"), (ToSide,"top"), (FromEnd,"arrow"),
                               (ToEnd,"none"), (Color,"#70A9EF")] {
            doc = db.edit_canvas(id, &doc, EdgeStyle { id:"line", field, value:Some(value) })?;
        }
        let edge = &doc.json()["edges"][0];
        assert_eq!(edge, &json!({"id":"line","fromNode":"a","toNode":"b","label":"字幕",
                                "extra":{"weight":3},"fromSide":"left","toSide":"top","fromEnd":"arrow",
                                "toEnd":"none","color":"#70A9EF"}));
        for (field, value) in [(FromSide,"diagonal"), (ToEnd,"circle"), (Color,"7")] {
            assert!(db.edit_canvas(id, &doc, EdgeStyle { id:"line", field, value:Some(value) }).is_err());
        }
        assert_eq!(db.read_canvas(id)?.as_str(), doc.as_str());
        doc = db.edit_canvas(id, &doc, EdgeStyle { id:"line", field:Color, value:Some("5") })?;
        doc = db.edit_canvas(id, &doc, EdgeStyle { id:"line", field:ToEnd, value:None })?;
        assert!(doc.json()["edges"][0].get("toEnd").is_none());
        assert_eq!(doc.json()["edges"][0]["color"], "5");
        Ok(())
    }

    #[test]
    fn stale_and_invalid_edits_do_not_write_and_noops_preserve_source_bytes() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-canvas-edit-{}", std::process::id()));
        std::fs::create_dir(&dir)?;
        let path = dir.join("notes.sqlite3");
        let first = Notebook::open(&path, false)?;
        let second = Notebook::open(&path, false)?;
        let raw = " {\n \"nodes\": [{\"id\":\"x\",\"type\":\"text\",\"text\":\"old\",\"x\":0,\"y\":0,\"width\":10,\"height\":10}] }\n";
        let old = canvas::parse(raw.to_owned())?;
        let id = first.save_canvas("畫布", &old)?;
        assert_eq!(first.edit_canvas(id, &old, ZOrder { id: "x", front: true })?.as_str(), raw);
        let concurrent = second.edit_canvas(id, &old, Content { id:"x", value:"第一個變更" })?;
        assert!(first.edit_canvas(id, &old, DeleteNode { id:"x" }).is_err());
        assert!(first.edit_canvas(id, &concurrent, AddEdge { from:"x", to:"missing", label:None }).is_err());
        let saved = first.read_canvas(id)?;
        assert_eq!(saved.as_str(), concurrent.as_str());
        let exported = dir.join("已編輯.canvas");
        canvas::export(&exported, &saved)?;
        assert_eq!(canvas::read(&exported)?.as_str(), concurrent.as_str());
        drop(second);
        drop(first);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
