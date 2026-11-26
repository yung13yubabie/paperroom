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
    GroupSelection { ids: &'a HashSet<String>, label: &'a str },
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

fn unique_id(tree: &Value, prefix: &str, reserved: &HashSet<String>) -> String {
    let ids: HashSet<&str> = ["nodes", "edges"].iter()
        .flat_map(|field| tree[*field].as_array().into_iter().flatten())
        .filter_map(|item| item["id"].as_str()).collect();
    let mut suffix = 1;
    loop {
        let id = format!("paperroom-{prefix}-{suffix}");
        if !ids.contains(id.as_str()) && !reserved.contains(&id) { return id; }
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

fn group_bounds(original: &Document, ids: &HashSet<String>) -> Result<[i64; 4]> {
    use crate::canvas_projection::{MAX_COORDINATE, all_geometry, encloses_geometry};

    const PADDING: i64 = 24;
    let mut selected = all_geometry(original).filter_map(|(node, rect)| {
        node["id"].as_str().filter(|id| ids.contains(*id)).map(|_| rect)
    });
    let [x, y, w, h] = selected.next().ok_or("請先框選要放入群組的節點。")?;
    let (mut left, mut top, mut right, mut bottom) = (x, y, x + w, y + h);
    let mut count = 1;
    for [x, y, w, h] in selected {
        left = left.min(x);
        top = top.min(y);
        right = right.max(x + w);
        bottom = bottom.max(y + h);
        count += 1;
    }
    if count != ids.len() {
        return Err("部分選取的節點已不存在或座標無法編輯。".into());
    }
    let bounds = [left - PADDING, top - PADDING,
        right - left + 2 * PADDING, bottom - top + 2 * PADDING];
    if bounds.iter().any(|v| v.abs() > MAX_COORDINATE) {
        return Err("選取範圍太大或太靠近畫布座標邊界。".into());
    }

    let groups = all_geometry(original).filter_map(|(node, rect)| {
        (node["type"].as_str() == Some("group")
            && node["id"].as_str().is_some_and(|id| ids.contains(id))).then_some(rect)
    }).collect::<Vec<_>>();
    for (node, rect) in all_geometry(original) {
        let is_group = node["type"].as_str() == Some("group");
        let already_inside = groups.iter().any(|group|
            (!is_group || *group != rect) && encloses_geometry(*group, rect));
        if encloses_geometry(bounds, rect) && !already_inside
            && !node["id"].as_str().is_some_and(|id| ids.contains(id))
        {
            return Err("這個群組也會包含未選取的節點。請調整框選範圍後再試。".into());
        }
    }
    Ok(bounds)
}

fn apply(original: &Document, edit: CanvasEdit<'_>, reserved: &HashSet<String>) -> Result<Document> {
    let mut tree = original.json().clone();
    match edit {
        CanvasEdit::AddNode { kind, content, geometry } => {
            let field = content_key(kind)?;
            let mut node = json!({"id":unique_id(&tree, "node", reserved), "type":kind});
            node[field] = text(content)?;
            set_geometry(&mut node, geometry);
            array(&mut tree, "nodes")?.push(node);
        }
        CanvasEdit::GroupSelection { ids, label } => {
            let label = label.trim();
            if label.is_empty() { return Err("請輸入群組名稱。".into()); }
            let geometry = group_bounds(original, ids)?;
            let mut group = json!({"id": unique_id(&tree, "node", reserved), "type": "group", "label": text(label)?});
            set_geometry(&mut group, geometry);
            let first = crate::canvas_projection::all_geometry(original)
                .find(|(_, rect)| crate::canvas_projection::encloses_geometry(geometry, *rect))
                .and_then(|(node, _)| node["id"].as_str()).ok_or("找不到選取的節點。")?;
            let nodes = array(&mut tree, "nodes")?;
            let index = position(nodes, first)?;
            nodes.insert(index, group);
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
            let mut edge = json!({"id":unique_id(&tree, "edge", reserved), "fromNode":from, "toNode":to});
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

const COMPANION_NOTES: &str =
    "SELECT p.note_id,p.relative_path FROM companion_canvases c
     JOIN vault_paths p ON p.import_root=c.import_root
     JOIN notes n ON n.id=p.note_id JOIN notes r ON r.id=p.import_root
     WHERE c.canvas_id=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL";

impl Notebook {
    pub(crate) fn edit_canvas(
        &self, canvas_id: i64, expected: &Document, edit: CanvasEdit<'_>,
    ) -> Result<Document> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let revised = edit_on(&tx, canvas_id, expected, edit)?;
        tx.commit()?;
        Ok(revised)
    }

    pub(crate) fn companion_note_choices(&self, id: i64) -> Result<Vec<(i64,String)>> {
        let mut stmt = self.connection.prepare(&format!("{COMPANION_NOTES} ORDER BY p.relative_path COLLATE BINARY"))?;
        let rows = stmt.query_map([id], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub(crate) fn add_companion_note_node(&self, id:i64, expected:&Document, note:i64, path:&str, geometry:[i64;4]) -> Result<Document> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let lookup = format!("{COMPANION_NOTES} AND p.note_id=?2 AND p.relative_path=?3 COLLATE BINARY");
        if !super::notion_handoff::windows_zip_path(path) ||
            tx.query_row(&lookup, params![id,note,path], |_| Ok(())).optional()?.is_none() {
            return Err("這篇同包筆記的來源已變更，請重新選取。".into());
        }
        let revised = edit_on(&tx,id,expected,CanvasEdit::AddNode{kind:"file",content:path,geometry})?;
        let node = revised.json()["nodes"].as_array().and_then(|nodes| nodes.last())
            .and_then(|n| n["id"].as_str()).ok_or("找不到新增的節點。")?;
        let inserted = tx.execute("INSERT INTO companion_node_sources(canvas_id,node_id,path,note_id)
            SELECT ?1,?2,?3,?4 WHERE (SELECT COUNT(*) FROM companion_node_sources WHERE canvas_id=?1)<15000",
            params![id,node,path,note])?;
        if inserted != 1 { return Err("這張畫布已達來源節點數量上限。".into()); }
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
        replace_source(&tx, canvas_id, expected.as_str(), restored.as_str())?;
        tx.commit()?;
        Ok(Some(restored))
    }
}

fn edit_on(tx:&Transaction<'_>, canvas_id:i64, expected:&Document, edit:CanvasEdit<'_>) -> Result<Document> {
    // Undo restores the original JSON and its source. A newly allocated
    // object must never inherit that source, even while the node is absent.
    let reserved = if matches!(&edit, CanvasEdit::AddNode {..} | CanvasEdit::GroupSelection {..} | CanvasEdit::AddEdge {..}) {
        let mut query = tx.prepare("SELECT node_id FROM companion_node_sources WHERE canvas_id=?1")?;
        query.query_map([canvas_id], |r| r.get(0))?.collect::<rusqlite::Result<HashSet<String>>>()?
    } else {
        HashSet::new()
    };
    let revised = apply(expected, edit, &reserved)?;
    replace_with_undo(tx,canvas_id,expected.as_str(),revised.as_str())?;
    Ok(revised)
}

pub(super) fn replace_with_undo(tx:&Transaction<'_>,canvas_id:i64,expected:&str,revised:&str)->Result<()> {
    replace_source(tx,canvas_id,expected,revised)?;
    if revised != expected {
        tx.execute("INSERT INTO canvas_edit_undo(canvas_id, source) VALUES (?1, ?2)",
            params![canvas_id, expected])?;
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
    Ok(())
}

fn replace_source(
    tx: &Transaction<'_>, canvas_id: i64, expected: &str, revised: &str,
) -> Result<()> {
    let changed = tx.execute(
        "UPDATE saved_canvases SET source=?3
         WHERE id=?1 AND source=?2
         AND length(CAST(?3 AS BLOB)) +
             (SELECT coalesce(sum(length(CAST(source AS BLOB))),0)
              FROM saved_canvases WHERE id<>?1) <= ?4",
        params![canvas_id, expected, revised, super::workspace::MAX_CANVAS_STORAGE_BYTES],
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
    fn explicitly_place_companion_notes_preserves_scope_reopen_and_undo() -> Result<()> {
        use crate::archive::ImportedPage;
        let dir = std::env::temp_dir().join(format!("paperroom-companion-placement-{}", std::process::id()));
        std::fs::create_dir(&dir)?;
        let file = dir.join("live.db");
        let backup = dir.join("copy.db");
        let mut db = Notebook::open(&file,false)?;
        let pages = || ["甲","乙"].into_iter().map(|folder| ImportedPage {
            parts:vec![folder.into(),"鏡頭.md".into()],title:"鏡頭".into(),body:folder.into()
        }).collect();
        let root = db.import_vault("本批",pages())?;
        let other = db.import_vault("另批",pages())?;
        db.connection.execute("INSERT INTO heptabase_import_roots VALUES(?1)", [root])?;
        let source = |r| db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path='甲/鏡頭.md'",[r],|row|row.get::<_,i64>(0));
        let (a,decoy) = (source(root)?,source(other)?);
        let raw = r#"{"nodes":[{"type":"group","id":"g","x":0,"y":0,"width":600,"height":400,"label":"分鏡"}]}"#;
        super::super::heptabase_boards::import_companion_boards(&db.connection,root,vec![("圖.canvas".into(),raw.into())],&Default::default())?;
        let id=db.saved_canvases()?[0].0;
        let original=db.read_canvas(id)?;
        let path="甲/鏡頭.md";
        let choices=db.companion_note_choices(id)?;
        assert_eq!(choices.len(),2);
        assert!(choices.contains(&(a,path.into())));
        let rect=[-250,80,232,114];
        assert!(db.add_companion_note_node(id,&original,decoy,path,rect).is_err());
        assert!(db.add_companion_note_node(id,&original,a,"甲/./鏡頭.md",rect).is_err());
        db.trash(a)?;
        assert!(db.add_companion_note_node(id,&original,a,path,rect).is_err());
        db.restore(a)?;
        let first=db.add_companion_note_node(id,&original,a,path,rect)?;
        assert!(db.add_companion_note_node(id,&original,a,path,rect).is_err());
        let second=db.add_companion_note_node(id,&first,a,path,[30,80,232,114])?;
        assert_eq!(second.json()["nodes"][0],original.json()["nodes"][0]);
        assert_eq!(db.canvas_note_placements(a)?[0].occurrences,2);
        assert!(db.canvas_note_placements(decoy)?.is_empty());
        db.backup(&backup)?;
        drop(db);
        for file in [&file,&backup] {
            let db=Notebook::open(file,false)?;
            assert_eq!(db.read_canvas(id)?.as_str(),second.as_str());
            assert!(db.canvas_file_links(id)?.iter().all(|link|link.note_id==Some(a)));
            let back=db.undo_canvas_edit(id,&second)?.unwrap();
            assert_eq!(back.as_str(),first.as_str());
            assert_eq!(db.canvas_note_placements(a)?[0].occurrences,1);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn new_file_node_cannot_reuse_a_deleted_companion_identity_but_undo_can() -> Result<()> {
        use crate::archive::ImportedPage;
        let mut db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let root = db.import_vault("合成來源", vec![ImportedPage {
            parts:vec!["甲.md".into()], title:"甲".into(), body:"原文".into(),
        }])?;
        db.connection.execute("INSERT INTO heptabase_import_roots VALUES(?1)", [root])?;
        let target: i64 = db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1", [root], |r|r.get(0))?;
        let original = "paperroom-node-1";
        let raw = json!({"nodes":[{"id":original,"type":"file","file":"甲.md","x":10,"y":20,"width":65,"height":80}]}).to_string();
        super::super::heptabase_boards::import_companion_boards(&db.connection,root,
            vec![("原圖.canvas".into(),raw.clone())], &[("甲.md".into(),target)].into_iter().collect())?;
        let id = db.saved_canvases()?[0].0;
        let doc = db.read_canvas(id)?;
        let deleted = db.edit_canvas(id,&doc,DeleteNode{id:original})?;
        let restored = db.undo_canvas_edit(id,&deleted)?.unwrap();
        assert_eq!(restored.as_str(),raw);
        assert_eq!(db.resolve_canvas_file_link(id,None,original)?.note_id,Some(target));
        let deleted = db.edit_canvas(id,&restored,DeleteNode{id:original})?;
        let replacement = db.edit_canvas(id,&deleted,AddNode{kind:"file",content:"甲.md",geometry:[10,20,65,80]})?;
        let new_id = replacement.json()["nodes"][0]["id"].as_str().unwrap();
        assert_ne!(new_id,original);
        assert_eq!(db.resolve_canvas_file_link(id,None,new_id)?.note_id,None);
        assert!(db.resolve_canvas_file_link(id,None,original).is_err());
        assert!(db.canvas_note_placements(target)?.is_empty());
        Ok(())
    }

    #[test]
    fn named_group_keeps_standard_fields_and_moves_only_enclosed_nodes() -> Result<()> {
        use crate::canvas_projection::move_nodes;
        let db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let original = canvas::parse(r#"{
            "nodes":[
                {"id":"child","type":"text","x":40,"y":40,"width":20,"height":20,"text":"原文"},
                {"id":"nested","type":"group","x":32,"y":32,"width":40,"height":42,"label":"內層","background":"assets/image.png","backgroundStyle":"ratio","vendor":true},
                {"id":"second","type":"text","x":80,"y":32,"width":20,"height":20,"text":"另一張"},
                {"id":"overlap","type":"text","x":100,"y":62,"width":60,"height":50,"text":"只相交"},
                {"id":"within-padding","type":"text","x":105,"y":35,"width":10,"height":10,"text":"靠近"}
            ],"edges":[{"id":"e","fromNode":"child","toNode":"overlap","label":"仍連得上"}],"vendor":{"root":true}
        }"#.to_owned())?;
        let id = db.save_canvas("群組驗收", &original)?;
        let selection = HashSet::from(["nested".to_owned(), "second".to_owned()]);
        for (ids, label) in [(&selection, " "), (&selection, "拒絕額外節點"),
            (&HashSet::new(), "空選取"), (&HashSet::from(["不存在".into()]), "過期選取")] {
            assert!(db.edit_canvas(id, &original, GroupSelection { ids, label }).is_err());
        }
        assert_eq!(db.read_canvas(id)?.as_str(), original.as_str());
        assert!(db.undo_canvas_edit(id, &original)?.is_none());
        let doc = db.edit_canvas(id, &original, Geometry { id:"within-padding", geometry:[160,35,10,10] })?;
        let grouped = db.edit_canvas(id, &doc, GroupSelection { ids: &selection, label:"外框 📌" })?;
        assert_eq!(grouped.json()["nodes"][0], json!({"id":"paperroom-node-1","type":"group",
            "x":8,"y":8,"width":116,"height":90,"label":"外框 📌"}));
        for field in ["edges","vendor"] { assert_eq!(grouped.json()[field], doc.json()[field]); }
        assert_eq!(&grouped.json()["nodes"].as_array().unwrap()[1..], doc.json()["nodes"].as_array().unwrap());
        let picked = HashSet::from(["paperroom-node-1".to_owned(), "child".to_owned()]);
        let (moves, _) = move_nodes(&grouped, &picked);
        assert_eq!(moves, ["paperroom-node-1","nested","child","second"].map(str::to_owned).into());
        let moved = db.edit_canvas(id, &grouped, Translate { ids: &moves, offset:[51,-20] })?;
        for (before, after) in grouped.json()["nodes"].as_array().unwrap().iter()
            .zip(moved.json()["nodes"].as_array().unwrap())
        {
            let mut expected = before.clone();
            if moves.contains(before["id"].as_str().unwrap()) {
                expected["x"] = json!(before["x"].as_i64().unwrap() + 51);
                expected["y"] = json!(before["y"].as_i64().unwrap() - 20);
            }
            assert_eq!(&expected, after);
        }
        assert_eq!(moved.json()["edges"], original.json()["edges"]);
        assert_eq!(db.undo_canvas_edit(id, &moved)?.unwrap().as_str(), grouped.as_str());
        Ok(())
    }

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
