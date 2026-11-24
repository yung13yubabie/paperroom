use super::{Notebook, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;
use std::collections::HashMap;

/// The selected source vault is local Paperroom state. It never changes the
/// user's Canvas JSON or associates a Canvas with a similarly named import.
pub(super) const SCHEMA: &str = "
CREATE TABLE canvas_vault_bindings (
    canvas_id INTEGER PRIMARY KEY REFERENCES saved_canvases(id) ON DELETE CASCADE,
    import_root INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE
);";

#[derive(Debug)]
pub struct CanvasFileLink {
    pub node_id: String,
    pub file: String,
    pub subpath: Option<String>,
    pub label: Option<String>,
    pub note_id: Option<i64>,
}

/// Interpret a file-node path relative to the selected vault root. This is
/// a lookup key only: no canonicalization against the OS, URL decoding, or I/O.
fn relative_key(file: &str) -> Option<String> {
    if file.is_empty()
        || file.starts_with(['/', '\\'])
        || file.ends_with(['/', '\\'])
        || file.contains(':')
        || file.chars().any(char::is_control)
    {
        return None;
    }
    let mut parts = Vec::new();
    let mut segments = file.split(['/', '\\']).peekable();
    while let Some(part) = segments.next() {
        match part {
            "." | ".." if segments.peek().is_none() => return None,
            "." | "" => {}
            ".." => { parts.pop()?; }
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

const ACTIVE_FILES: &str =
    "SELECT p.relative_path,p.note_id FROM canvas_vault_bindings b
     JOIN vault_paths p ON p.import_root=b.import_root
     JOIN notes n ON n.id=p.note_id
     JOIN notes r ON r.id=b.import_root
     WHERE b.canvas_id=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL";

fn file_link(node: &serde_json::Value) -> Result<CanvasFileLink> {
    Ok(CanvasFileLink {
        node_id: node["id"].as_str().ok_or("Canvas 檔案節點缺少 ID。")?.to_owned(),
        file: node["file"].as_str().ok_or("Canvas 檔案節點缺少路徑。")?.to_owned(),
        subpath: node.get("subpath").and_then(|v| v.as_str()).map(str::to_owned),
        label: node.get("label").and_then(|v| v.as_str()).map(str::to_owned),
        note_id: None,
    })
}

#[derive(PartialEq, Eq)]
pub(super) struct CanvasRelocation {
    pub id: i64,
    pub title: String,
    pub nodes: Vec<String>,
    pub block_references: Vec<(String, String)>,
    pub original: String,
    pub revised: String,
}

pub(super) fn plan_vault_relocation(
    conn: &Connection, import_root: i64, old_path: &str, new_path: &str,
) -> Result<Vec<CanvasRelocation>> {
    let mut stmt = conn.prepare(
        "SELECT c.id,c.title,c.source,bundle.canvas_id IS NOT NULL FROM canvas_vault_bindings b
         JOIN saved_canvases c ON c.id=b.canvas_id
         LEFT JOIN canvas_bundles bundle ON bundle.canvas_id=c.id
         WHERE b.import_root=?1 ORDER BY c.id",
    )?;
    let mut rows = stmt.query([import_root])?;
    let mut edits = Vec::new();
    let mut size_delta = 0i64;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let title: String = row.get(1)?;
        let raw = row.get(2)?;
        let document = if row.get::<_, bool>(3)? {
            crate::canvas::parse_bundle(raw)?
        } else {
            crate::canvas::parse(raw)?
        };
        let mut changes = Vec::new();
        let mut block_references = Vec::new();
        for (index, node) in document.json()["nodes"].as_array().into_iter().flatten().enumerate() {
            if node["type"].as_str() != Some("file") { continue; }
            let link = file_link(node)?;
            let key = relative_key(&link.file);
            if key.as_deref() == Some(new_path) {
                return Err(format!(
                    "Canvas「{title}」(#{id}) 的節點「{}」已指向新路徑，須先釐清其來源才能避免誤綁。",
                    link.node_id,
                ).into());
            }
            if key.as_deref() == Some(old_path) {
                if let Some(block) = link.subpath.as_deref().and_then(|fragment| fragment.strip_prefix("#^")) {
                    block_references.push((link.node_id.clone(), block.to_owned()));
                }
                changes.push((index, link.node_id));
            }
        }
        if changes.is_empty() { continue; }

        // The existing path resolver identifies the active note via the
        // selected import and the unique old path. Change only the file key.
        let mut tree = document.json().clone();
        for (index, _) in &changes {
            tree["nodes"][*index]["file"] = Value::String(new_path.to_owned());
        }
        let revised = super::canvas_edit::serialize_edit(&document, tree)?;
        size_delta += revised.as_str().len() as i64 - document.as_str().len() as i64;
        edits.push(CanvasRelocation {
            id,
            title,
            nodes: changes.into_iter().map(|(_, node)| node).collect(),
            block_references,
            original: document.as_str().to_owned(),
            revised: revised.as_str().to_owned(),
        });
    }
    if size_delta > 0 && !conn.query_row(
        "SELECT coalesce(sum(length(CAST(source AS BLOB))),0)+?1 <= ?2 FROM saved_canvases",
        [size_delta, super::workspace::MAX_CANVAS_STORAGE_BYTES], |row| row.get::<_, bool>(0),
    )? {
        return Err("新路徑會使工作區的 Canvas 總容量超過 64 MiB。".into());
    }
    Ok(edits)
}

impl Notebook {
    pub fn imported_vaults(&self) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.connection.prepare(
            "SELECT r.id,r.title FROM notes r WHERE r.trash_root IS NULL
             AND EXISTS (SELECT 1 FROM vault_paths p WHERE p.import_root=r.id)
             ORDER BY r.title,r.id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_canvas_vault(&self, canvas_id: i64, import_root: Option<i64>) -> Result<()> {
        self.compare_set_canvas_vault(canvas_id, self.canvas_vault(canvas_id)?, import_root)
    }

    pub fn compare_set_canvas_vault(
        &self, canvas_id: i64, expected: Option<i64>, next: Option<i64>,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        self.check_canvas_vault(canvas_id, expected)?;
        if let Some(root) = next {
            let changed = tx.execute(
                "INSERT INTO canvas_vault_bindings(canvas_id, import_root)
                 SELECT c.id,r.id FROM saved_canvases c,notes r
                 WHERE c.id=?1 AND r.id=?2 AND r.trash_root IS NULL
                   AND EXISTS (SELECT 1 FROM vault_paths p WHERE p.import_root=r.id)
                 ON CONFLICT(canvas_id) DO UPDATE SET import_root=excluded.import_root",
                params![canvas_id, root],
            )?;
            if changed == 0 { return Err("找不到 Canvas 或可使用的來源筆記庫。".into()); }
        } else {
            tx.execute("DELETE FROM canvas_vault_bindings WHERE canvas_id=?1", [canvas_id])?;
        }
        tx.commit()?;
        Ok(())
    }

    // Call from a transaction on this connection so the comparison and the
    // subsequent read or write see the same binding.
    fn check_canvas_vault(&self, canvas_id: i64, expected: Option<i64>) -> Result<()> {
        if self.canvas_vault(canvas_id)? != expected {
            return Err("Canvas 的來源筆記庫已變更，請重新開啟選單。".into());
        }
        Ok(())
    }

    pub fn canvas_vault(&self, canvas_id: i64) -> Result<Option<i64>> {
        self.connection.query_row(
            "SELECT b.import_root FROM saved_canvases c
             LEFT JOIN canvas_vault_bindings b ON b.canvas_id=c.id WHERE c.id=?1",
            [canvas_id], |row| row.get(0),
        ).optional()?.ok_or_else(|| "找不到 Canvas。".into())
    }

    pub fn canvas_file_links(&self, canvas_id: i64) -> Result<Vec<CanvasFileLink>> {
        Ok(self.canvas_file_link_snapshot(canvas_id)?.1)
    }

    pub fn canvas_file_link_snapshot(&self, canvas_id: i64) -> Result<(Option<i64>, Vec<CanvasFileLink>)> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        let scope = self.canvas_vault(canvas_id)?;
        let doc = self.read_canvas(canvas_id)?;
        let notes = {
            let mut stmt = tx.prepare(ACTIVE_FILES)?;
            let rows = stmt.query_map([canvas_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            rows.collect::<rusqlite::Result<HashMap<_, _>>>()?
        };
        let mut links = Vec::new();
        for node in doc.json()["nodes"].as_array().into_iter().flatten() {
            if node["type"].as_str() != Some("file") { continue; }
            let mut link = file_link(node)?;
            link.note_id = relative_key(&link.file).and_then(|key| notes.get(&key).copied());
            links.push(link);
        }
        tx.commit()?;
        Ok((scope, links))
    }

    pub fn resolve_canvas_file_link(
        &self, canvas_id: i64, expected: Option<i64>, node_id: &str,
    ) -> Result<CanvasFileLink> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        self.check_canvas_vault(canvas_id, expected)?;
        let doc = self.read_canvas(canvas_id)?;
        let selected = doc.json()["nodes"].as_array().into_iter().flatten().find(|node| {
            node["type"].as_str() == Some("file") && node["id"].as_str() == Some(node_id)
        }).ok_or("找不到選取的 Canvas 檔案節點，請重新開啟選單。")?;
        let mut link = file_link(selected)?;
        if let Some(path) = relative_key(&link.file) {
            let lookup = format!("{ACTIVE_FILES} AND p.relative_path=?2");
            link.note_id = tx.query_row(&lookup, params![canvas_id, path], |r| r.get(1)).optional()?;
        }
        tx.commit()?;
        Ok(link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use std::path::Path;

    #[test]
    fn file_nodes_bind_to_exact_paths_in_one_import_and_keep_distinct_ids() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let page = |dir: &str, text: &str| ImportedPage {
            parts: vec![dir.into(), "筆記.md".into()], title: "筆記".into(), body: text.into(),
        };
        let first = db.import_vault("同名庫", vec![page("資料", "第一庫"), page("其他", "同名頁")])?;
        let second = db.import_vault("同名庫", vec![page("資料", "第二庫")])?;
        let files = [
            ("first", "資料/筆記.md", "#項目", "分組一"),
            ("second", r".\其他\..\資料\筆記.md", "#^章節", "分組二"),
            ("ambiguous", "筆記.md", "#項目", "同名"),
            ("outside", "../資料/筆記.md", "#項目", "外部"),
            ("remote", "https://example.invalid/筆記.md", "#項目", "連結"),
        ];
        let nodes: Vec<_> = files.iter().map(|(id, file, subpath, label)| serde_json::json!({
            "type":"file", "id":id, "x":0, "y":0, "width":10, "height":10,
            "file":file, "subpath":subpath, "label":label
        })).collect();
        let raw = serde_json::json!({"nodes":nodes}).to_string();
        let canvas = db.save_canvas("測試", &crate::canvas::parse(raw.clone())?)?;
        assert_eq!(db.imported_vaults()?.len(), 2);
        assert!(db.canvas_file_links(canvas)?.iter().all(|n| n.note_id.is_none()));
        db.set_canvas_vault(canvas, Some(first))?;
        let links = db.canvas_file_links(canvas)?;
        assert_eq!(links.len(), 5);
        let id = links[0].note_id.unwrap();
        assert_eq!(links[1].note_id, Some(id));
        assert_eq!(db.note(id)?.unwrap().body, "第一庫");
        for (link, (node_id, file, subpath, label)) in links.iter().zip(files) {
            assert_eq!(link.node_id, node_id);
            assert_eq!(link.file, file);
            assert_eq!(link.subpath.as_deref(), Some(subpath));
            assert_eq!(link.label.as_deref(), Some(label));
        }
        assert!(links[2..].iter().all(|n| n.note_id.is_none()));
        db.trash(id)?;
        assert!(db.canvas_file_links(canvas)?[0].note_id.is_none());
        db.restore(id)?;
        assert_eq!(db.canvas_file_links(canvas)?[0].note_id, Some(id));
        db.set_canvas_vault(canvas, Some(second))?;
        let other_id = db.canvas_file_links(canvas)?[0].note_id.unwrap();
        assert_ne!(other_id, id);
        assert_eq!(db.note(other_id)?.unwrap().body, "第二庫");
        assert!(db.set_canvas_vault(canvas, Some(id)).is_err());
        assert_eq!(db.canvas_vault(canvas)?, Some(second));
        assert_eq!(db.read_canvas(canvas)?.as_str(), raw);
        db.set_canvas_vault(canvas, None)?;
        assert!(db.canvas_file_links(canvas)?.iter().all(|n| n.note_id.is_none()));
        Ok(())
    }

    #[test]
    fn a_second_connection_cannot_redirect_a_pending_selection_or_binding_change() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-canvas-cas-{}", std::process::id()));
        std::fs::create_dir(&dir)?;
        let path = dir.join("notes.sqlite3");
        let mut first = Notebook::open(&path, false)?;
        let second = Notebook::open(&path, false)?;
        let page = || ImportedPage {
            parts: vec!["Note.md".into()], title: "Note".into(), body: "合成筆記".into(),
        };
        let a = first.import_vault("A", vec![page()])?;
        let b = first.import_vault("B", vec![page()])?;
        let json = r#"{"nodes":[{"id":"f","type":"file","file":"Note.md","x":0,"y":0,"width":1,"height":1}]}"#;
        let id = first.save_canvas("畫布", &crate::canvas::parse(json.into())?)?;
        first.compare_set_canvas_vault(id, None, Some(a))?;
        let (scope, links) = first.canvas_file_link_snapshot(id)?;
        let old_target = links[0].note_id.unwrap();
        second.compare_set_canvas_vault(id, Some(a), Some(b))?;
        assert!(first.resolve_canvas_file_link(id, scope, "f").is_err());
        assert!(first.compare_set_canvas_vault(id, scope, None).is_err());
        assert!(first.compare_set_canvas_vault(id, scope, Some(a)).is_err());
        assert_eq!(first.canvas_vault(id)?, Some(b));
        let fresh_target = first.resolve_canvas_file_link(id, Some(b), "f")?.note_id.unwrap();
        assert_ne!(fresh_target, old_target);
        second.trash(fresh_target)?;
        assert!(first.resolve_canvas_file_link(id, Some(b), "f")?.note_id.is_none());
        second.compare_set_canvas_vault(id, Some(b), None)?;
        assert!(first.resolve_canvas_file_link(id, Some(b), "f").is_err());
        assert!(first.resolve_canvas_file_link(id, None, "missing").is_err());
        drop(second);
        drop(first);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn absolute_external_and_above_root_paths_remain_unresolved() {
        for path in ["/筆記.md", r"C:\筆記.md", r"\\host\筆記.md", "file:///a.md",
                     "a/../../筆記.md", "note.md/.", "", "筆記.md/"] {
            assert!(relative_key(path).is_none(), "{path}");
        }
        assert_eq!(relative_key("./甲/../乙/筆記.md").as_deref(), Some("乙/筆記.md"));
    }
}
