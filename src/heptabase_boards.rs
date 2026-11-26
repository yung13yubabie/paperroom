//! Convert representable companion JSON Canvas drawings into native boards.
use super::{boards::create_board_on, notion_handoff::windows_zip_path, Result, MAX_TITLE_CHARS};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub(super) const SOURCE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS companion_canvases(
    canvas_id INTEGER PRIMARY KEY REFERENCES saved_canvases(id) ON DELETE CASCADE,
    import_root INTEGER REFERENCES heptabase_import_roots(import_root) ON DELETE SET NULL
);
CREATE TABLE IF NOT EXISTS companion_node_sources(
    canvas_id INTEGER NOT NULL REFERENCES companion_canvases(canvas_id) ON DELETE CASCADE,
    node_id TEXT NOT NULL, path TEXT NOT NULL,
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    PRIMARY KEY(canvas_id,node_id)
);
";

struct Card { note: i64, x: i64, y: i64 }
struct Layout { cards: Vec<Card>, edges: Vec<(i64, i64)> }

fn only_fields(value: &Value, allowed: &[&str]) -> bool {
    value.as_object().is_some_and(|fields| fields.keys().all(|key| allowed.contains(&key.as_str())))
}

fn layout(document: &crate::canvas::Document, notes: &HashMap<String, i64>) -> Result<Layout> {
    let root = document.json();
    if !only_fields(root, &["nodes", "edges"]) {
        return Err("畫布含有無法轉入原生白板的額外欄位。".into());
    }
    if document.nodes > 128 { return Err("原生白板最多可放 128 張卡片。".into()); }
    let mut cards = Vec::<Card>::with_capacity(document.nodes);
    let mut ids = HashMap::new();
    let mut placed = HashSet::new();
    for node in root["nodes"].as_array().into_iter().flatten() {
        if node["type"] != "file" || !only_fields(node, &["id", "type", "file", "x", "y", "width", "height"]) {
            return Err("僅支援指向完整 Markdown 筆記的普通檔案節點，無法保留此節點類型或附加欄位。".into());
        }
        let id = node["id"].as_str().filter(|id| !id.is_empty()).ok_or("節點缺少識別碼。")?;
        let path = node["file"].as_str().ok_or("節點缺少檔案路徑。")?;
        if !windows_zip_path(path) || !path.rsplit_once('.').is_some_and(|(_, ext)| {
            ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
        }) {
            return Err("檔案節點必須指定 ZIP 根目錄內的 Markdown 路徑。".into());
        }
        let &note = notes.get(path).ok_or_else(|| format!("這次匯入沒有符合此完整路徑的筆記：{path}"))?;
        if !placed.insert(note) { return Err("同一張白板多次放置同一篇筆記，無法保留所有位置。".into()); }
        let x = node["x"].as_i64().filter(|v| (0..=10000).contains(v)).ok_or("卡片水平座標超出原生白板範圍。")?;
        let y = node["y"].as_i64().filter(|v| (0..=50000).contains(v)).ok_or("卡片垂直座標超出原生白板範圍。")?;
        if node["width"].as_i64() != Some(232) || node["height"].as_i64() != Some(114) {
            return Err("普通筆記卡片尺寸必須為 232 × 114。".into());
        }
        // Native cards paint in note-ID order; preserve the visible card and hit target.
        if cards.iter().any(|prior| prior.note > note && (prior.x-x).abs() < 232 && (prior.y-y).abs() < 114) {
            return Err("重疊卡片的前後順序無法保留。".into());
        }
        ids.insert(id, note);
        cards.push(Card {note, x, y});
    }
    let mut edges = Vec::with_capacity(document.edges);
    let mut seen = HashSet::new();
    for edge in root["edges"].as_array().into_iter().flatten() {
        // JSON Canvas defaults fromEnd to none and toEnd to arrow.
        if !only_fields(edge, &["id", "fromNode", "toNode", "fromEnd", "toEnd"])
            || edge["toEnd"] != "none" || edge.get("fromEnd").is_some_and(|end| end != "none")
        {
            return Err("原生白板僅支援無箭頭、無標籤且未指定連接邊的普通連線。".into());
        }
        let endpoint = |key: &str| edge[key].as_str().and_then(|id| ids.get(id)).copied().ok_or("找不到連線端點。");
        let a = endpoint("fromNode")?;
        let b = endpoint("toNode")?;
        let pair = (a.min(b), a.max(b));
        if a == b || !seen.insert(pair) { return Err("原生白板無法保留自我連線或重複連線。".into()); }
        edges.push(pair);
    }
    Ok(Layout {cards, edges})
}

#[derive(Default)]
pub(crate) struct CompanionReport {
    pub(crate) boards: usize,
    pub(crate) canvases: usize,
    pub(crate) skipped: usize,
    pub(crate) details: Vec<String>,
}

#[cfg(test)]
pub(super) fn import_companion_boards(
    db: &Connection, import_root: i64, documents: Vec<(String, String)>, notes: &HashMap<String, i64>,
) -> Result<CompanionReport> {
    import_companion_with_images(db,import_root,documents,notes,&Default::default())
}

pub(super) fn import_companion_with_images(
    db:&Connection, import_root:i64, documents:Vec<(String,String)>, notes:&HashMap<String,i64>,
    images:&std::collections::BTreeMap<String,Vec<u8>>,
) -> Result<CompanionReport> {
    let mut report = CompanionReport::default();
    for (member, raw) in documents {
        let filename = member.rsplit('/').next().unwrap_or("");
        let title = filename.rsplit_once('.').filter(|(_, ext)| ext.eq_ignore_ascii_case("canvas"))
            .map(|(stem, _)| stem).unwrap_or("");
        if !windows_zip_path(&member) || title.is_empty() || title.trim() != title || title.chars().count() > MAX_TITLE_CHARS {
            report.skipped += 1;
            report.details.push(format!("{member}：已略過，白板檔名或路徑無法保留。"));
            continue;
        }
        let document = match crate::canvas::parse(raw) {
            Ok(document) => document,
            Err(error) => {
                report.skipped += 1;
                report.details.push(format!("{member}：已略過，JSON Canvas 無法讀取。{error}"));
                continue;
            }
        };
        let native = layout(&document, notes);
        let reason = match &native {
            Err(error) => Some(error.to_string()),
            Ok(_) if db.query_row("SELECT EXISTS(SELECT 1 FROM boards WHERE title=?1)", [title], |r| r.get::<_, bool>(0))? =>
                Some("已有同名原生白板。".to_owned()),
            _ => None,
        };
        if let Some(reason) = reason {
            // Store original node identities separately from ordinary Vault bindings.
            let id = super::workspace::save_canvas_on(db, &member, &document)?;
            db.execute("INSERT INTO companion_canvases(canvas_id,import_root) VALUES(?1,?2)", [id,import_root])?;
            let mut bound = 0;
            let mut missing = 0;
            for node in document.json()["nodes"].as_array().into_iter().flatten().filter(|n| n["type"]=="file") {
                let path = node["file"].as_str().ok_or("檔案節點缺少路徑。")?;
                if let Some(note) = notes.get(path).filter(|_| windows_zip_path(path)) {
                    db.execute("INSERT INTO companion_node_sources(canvas_id,node_id,path,note_id) VALUES(?1,?2,?3,?4)",
                        params![id,node["id"].as_str(),path,note])?;
                    bound += 1;
                } else {
                    missing += 1;
                }
            }
            let (kept,unavailable) = super::canvas_bundle::attach_companion_images(db,id,&member,&document,images)?;
            if kept + unavailable > 0 {
                report.details.push(format!("{member}：已保存 {kept} 份同包圖片；{unavailable} 份圖片參照未取得。"));
            }
            report.canvases += 1;
            report.details.push(format!("{member}：已保存為可編輯 Canvas（#{id}）。同包來源 {bound} 個，未對應檔案節點 {missing} 個。{reason}"));
            continue;
        }
        let layout = native?;
        let board = create_board_on(db, title)?;
        let mut card = db.prepare_cached("INSERT INTO board_cards(board_id,note_id,x,y) VALUES(?1,?2,?3,?4)")?;
        for c in layout.cards { card.execute(params![board,c.note,c.x,c.y])?; }
        let mut edge = db.prepare_cached("INSERT INTO board_edges(board_id,from_note,to_note) VALUES(?1,?2,?3)")?;
        for (a,b) in layout.edges { edge.execute(params![board,a,b])?; }
        report.boards += 1;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn drawing() -> Value {
        json!({"nodes":[
            {"id":"one","type":"file","file":"甲/鏡頭.md","width":232,"height":114,"x":10,"y":20},
            {"id":"two","type":"file","file":"乙/鏡頭.md","width":232,"height":114,"x":300,"y":120}
        ],"edges":[{"id":"line","fromNode":"one","toNode":"two","toEnd":"none"}]})
    }

    #[test]
    fn companion_boards_share_exact_notes_and_keep_editable_drawings() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE saved_canvases(id INTEGER PRIMARY KEY,title TEXT NOT NULL,source TEXT NOT NULL);
            CREATE TABLE boards(id INTEGER PRIMARY KEY,title TEXT COLLATE NOCASE UNIQUE);
            CREATE TABLE board_cards(board_id INTEGER,note_id INTEGER,x INTEGER,y INTEGER,PRIMARY KEY(board_id,note_id));
            CREATE TABLE board_edges(board_id INTEGER,from_note INTEGER,to_note INTEGER,PRIMARY KEY(board_id,from_note,to_note));")?;
        db.execute_batch("CREATE TABLE heptabase_import_roots(import_root INTEGER PRIMARY KEY); INSERT INTO heptabase_import_roots VALUES(1); CREATE TABLE notes(id INTEGER PRIMARY KEY); INSERT INTO notes VALUES(42),(7);")?;
        db.execute_batch(SOURCE_SCHEMA)?;
        let tx = db.transaction()?;
        let notes = HashMap::from([("甲/鏡頭.md".to_owned(), 42), ("乙/鏡頭.md".to_owned(), 7)]);
        let first = drawing();
        let mut second = drawing();
        second["nodes"][0]["x"] = 600.into();
        let mut arrow = drawing();
        arrow["edges"][0].as_object_mut().unwrap().remove("toEnd");
        let mut repeated = drawing();
        repeated["nodes"][1]["file"] = "甲/鏡頭.md".into();
        let mut ambiguous = drawing();
        ambiguous["nodes"][0]["file"] = "鏡頭.md".into();
        let mut unknown = drawing();
        unknown["nodes"][0]["extension"] = true.into();
        let mut overlap = drawing();
        overlap["nodes"][1]["x"] = 10.into();
        let mut labelled = drawing();
        labelled["edges"][0]["label"] = "有意義的邊".into();
        let mut group = drawing();
        group["nodes"][1]["type"] = "group".into();
        let inputs = [("分鏡",first),("參考",second),("箭頭",arrow),("重放",repeated),("缺件",ambiguous),
            ("擴充",unknown),("疊放",overlap),("標籤",labelled),("群組",group)];
        let report = import_companion_boards(&tx, 1, inputs.into_iter()
            .map(|(name, doc)| (format!("手稿/{name}.canvas"),doc.to_string())).collect(), &notes)?;
        assert_eq!(report.boards, 2);
        assert_eq!(report.canvases, 7);
        assert_eq!(report.details.len(), 7);
        assert!(report.details.iter().all(|s| s.starts_with("手稿/") && s.contains(".canvas：")));
        assert_eq!(tx.query_row("SELECT count(*) FROM boards", [], |r| r.get::<_, i64>(0))?, 2);
        assert_eq!(tx.query_row("SELECT count(*) FROM board_edges WHERE from_note=7 AND to_note=42", [], |r| r.get::<_, i64>(0))?, 2);
        let positions = tx.prepare("SELECT note_id,x,y FROM board_cards ORDER BY board_id,note_id")?
            .query_map([], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        assert_eq!(positions, [(7,300,120),(42,10,20),(7,300,120),(42,600,20)]);
        Ok(())
    }
}
