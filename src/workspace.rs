use super::{Notebook, Result, properties};
use crate::{canvas, canvas_projection::Camera, formula};
use rusqlite::{OptionalExtension, params};

pub(super) const MAX_CANVAS_STORAGE_BYTES: i64 = 64 * 1024 * 1024;

pub(super) const SCHEMA: &str = "
CREATE TABLE saved_canvases(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    source TEXT NOT NULL
);
CREATE TABLE collection_formulas(
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    expression TEXT NOT NULL,
    PRIMARY KEY(collection_id,name)
);
";

pub(super) const CAMERA_SCHEMA: &str = "
CREATE TABLE canvas_cameras(
    canvas_id INTEGER PRIMARY KEY REFERENCES saved_canvases(id) ON DELETE CASCADE,
    x REAL NOT NULL CHECK(x BETWEEN -4e15 AND 4e15),
    y REAL NOT NULL CHECK(y BETWEEN -4e15 AND 4e15),
    scale REAL NOT NULL CHECK(scale BETWEEN 1e-16 AND 12.0)
);
";

pub(super) fn save_canvas_on(db: &rusqlite::Connection, title: &str, canvas: &canvas::Document) -> Result<i64> {
    let title = super::clean_title(title);
    let changed = db.execute(
        "INSERT INTO saved_canvases(title,source)
         SELECT ?1,?2
         WHERE (SELECT count(*) FROM saved_canvases)<128
         AND (SELECT coalesce(sum(length(CAST(source AS BLOB))),0) FROM saved_canvases)
             +length(CAST(?2 AS BLOB)) <= ?3",
        params![title, canvas.as_str(), MAX_CANVAS_STORAGE_BYTES],
    )?;
    if changed != 1 { return Err("已達工作區 Canvas 儲存上限。".into()); }
    Ok(db.last_insert_rowid())
}

impl Notebook {
    pub(crate) fn canvas_camera(&self, id: i64) -> Result<Option<Camera>> {
        let camera = self.connection.query_row(
            "SELECT x,y,scale FROM canvas_cameras WHERE canvas_id=?1", [id],
            |row| Ok(Camera { origin: [row.get(0)?, row.get(1)?], scale: row.get(2)? }),
        ).optional()?;
        Ok(camera.filter(|camera| camera.valid()))
    }

    pub(crate) fn set_canvas_camera(&self, id: i64, camera: Option<Camera>) -> Result<()> {
        if let Some(camera) = camera {
            if !camera.valid() { return Err("畫布視野超出可保存範圍。".into()); }
            self.connection.execute(
                "INSERT INTO canvas_cameras(canvas_id,x,y,scale) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(canvas_id) DO UPDATE SET x=excluded.x,y=excluded.y,scale=excluded.scale",
                params![id, camera.origin[0], camera.origin[1], camera.scale],
            )?;
        } else {
            self.connection.execute("DELETE FROM canvas_cameras WHERE canvas_id=?1", [id])?;
        }
        Ok(())
    }

    pub fn save_canvas(&self, title: &str, canvas: &canvas::Document) -> Result<i64> {
        save_canvas_on(&self.connection, title, canvas)
    }

    pub fn saved_canvases(&self) -> Result<Vec<(i64, String)>> {
        let mut query = self.connection.prepare("SELECT id,title FROM saved_canvases ORDER BY id")?;
        let rows = query.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn read_canvas(&self, id: i64) -> Result<canvas::Document> {
        let (source, bundled) = self.connection.query_row(
            "SELECT c.source,b.canvas_id IS NOT NULL FROM saved_canvases c
             LEFT JOIN canvas_bundles b ON b.canvas_id=c.id WHERE c.id=?1",
            [id], |row| Ok((row.get(0)?, row.get::<_, bool>(1)?)),
        ).optional()?.ok_or("找不到已匯入的 Canvas。")?;
        if bundled { canvas::parse_bundle(source) } else { canvas::parse(source) }
    }

    pub fn remove_canvas(&self, id: i64, expected: &canvas::Document) -> Result<()> {
        let changed = self.connection.execute(
            "DELETE FROM saved_canvases WHERE id=?1 AND source=?2",
            params![id, expected.as_str()],
        )?;
        if changed != 1 {
            return Err("畫布已更改或移除。請重新開啟畫布。".into());
        }
        Ok(())
    }

    pub fn add_collection_formula(&self, collection: i64, name: &str, expression: &str) -> Result<()> {
        let name = properties::key_checked(name)?;
        formula::validate(expression)?;
        let changed = self.connection.execute(
            "INSERT INTO collection_formulas(collection_id,name,expression)
             SELECT ?1,?2,?3 FROM collections WHERE id=?1
             AND (SELECT count(*) FROM collection_formulas WHERE collection_id=?1)<32
             ON CONFLICT(collection_id,name) DO NOTHING",
            params![collection, name, expression],
        )?;
        if changed != 1 {
            return Err("找不到集合、公式名稱已使用，或已達 32 條公式。".into());
        }
        Ok(())
    }

    pub fn edit_collection_formula(
        &self, collection: i64, name: &str, expected: &str, next: &str,
    ) -> Result<()> {
        let name = properties::key_checked(name)?;
        formula::validate(next)?;
        let changed = self.connection.execute(
            "UPDATE collection_formulas SET expression=?4
             WHERE collection_id=?1 AND name=?2 AND expression=?3",
            params![collection, name, expected, next],
        )?;
        if changed != 1 {
            return Err("公式已被另一個視窗變更或移除，請重新開啟集合。".into());
        }
        Ok(())
    }

    pub fn remove_collection_formula(&self, collection: i64, name: &str, expected: &str) -> Result<()> {
        let changed = self.connection.execute(
            "DELETE FROM collection_formulas WHERE collection_id=?1 AND name=?2 AND expression=?3",
            params![collection, name, expected],
        )?;
        if changed != 1 { return Err("公式已更改，請重新開啟集合。".into()); }
        Ok(())
    }

    pub fn collection_formulas(&self, collection: i64) -> Result<Vec<(String, String)>> {
        let mut query = self.connection.prepare(
            "SELECT name,expression FROM collection_formulas WHERE collection_id=?1 ORDER BY name",
        )?;
        let rows = query.query_map([collection], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    #[test]
    fn canvas_view_upgrades_v26_reopens_and_backs_up_without_editing_json() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("views.db");
        let backup = dir.join("backup.db");
        let raw = r#"{"nodes":[{"id":"p","type":"text","x":4,"y":-2,"width":20,"height":15,"text":"圖"}],"edges":[]}"#;
        let (id, other) = {
            let legacy = Notebook::open(&path, false)?;
            let doc = canvas::parse(raw.into())?;
            let id = legacy.save_canvas("畫布", &doc)?;
            let other = legacy.save_canvas("第二份", &doc)?;
            super::super::tests::remove_v27_schema(&legacy.connection)?;
            legacy.connection.pragma_update(None, "user_version", 26)?;
            (id, other)
        };
        let camera = Camera { origin: [12.25, -90.5], scale: 1.125 };
        {
            let upgraded = Notebook::open(&path, false)?;
            assert_eq!(upgraded.canvas_camera(id)?, None);
            upgraded.set_canvas_camera(id, Some(camera))?;
            assert_eq!(upgraded.canvas_camera(other)?, None);
            assert!(upgraded.set_canvas_camera(id, Some(Camera { origin: [f64::NAN, 0.0], scale: 1.0 })).is_err());
            assert_eq!(upgraded.canvas_camera(id)?, Some(camera));
            upgraded.backup(&backup)?;
        }
        for path in [&path, &backup] {
            let db = Notebook::open(path, false)?;
            assert_eq!(db.canvas_camera(id)?, Some(camera));
            assert_eq!(db.read_canvas(id)?.as_str(), raw);
            db.set_canvas_camera(id, None)?;
            assert_eq!(db.canvas_camera(id)?, None);
            db.set_canvas_camera(id, Some(camera))?;
            db.remove_canvas(id, &db.read_canvas(id)?)?;
            assert_eq!(db.canvas_camera(id)?, None);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn saved_canvas_preserves_bytes_and_formulas_are_collection_scoped() -> Result<()> {
        let notebook = Notebook::open(Path::new(":memory:"), false)?;
        let source = "{\r\n  \"nodes\": [], \"appNote\": \"中文\" }\r\n";
        let doc = canvas::parse(source.to_owned())?;
        let id = notebook.save_canvas("資料圖", &doc)?;
        assert_eq!(notebook.read_canvas(id)?.as_str(), source);
        assert_eq!(notebook.saved_canvases()?, [(id, "資料圖".into())]);
        let collection = notebook.create_collection("研究")?;
        let another = notebook.create_collection("會議")?;
        notebook.add_collection_formula(collection, "總計", "1 + 2")?;
        assert!(notebook.add_collection_formula(collection, "總計", "2 + 2").is_err());
        assert!(notebook.add_collection_formula(collection, "語法錯誤", "1 +").is_err());
        assert_eq!(notebook.collection_formulas(collection)?, [("總計".into(), "1 + 2".into())]);
        assert!(notebook.collection_formulas(another)?.is_empty());
        assert!(notebook.remove_collection_formula(collection, "總計", "1 + 1").is_err());
        assert!(notebook.edit_collection_formula(collection, "總計", "1+2", "2 + 2").is_err());
        assert!(notebook.edit_collection_formula(collection, "總計", "1 + 2", "2 +").is_err());
        notebook.edit_collection_formula(collection, "總計", "1 + 2", "2 + 2")?;
        assert!(notebook.edit_collection_formula(collection, "總計", "1 + 2", "2 + 3").is_err());
        assert_eq!(notebook.collection_formulas(collection)?, [("總計".into(), "2 + 2".into())]);
        notebook.remove_collection_formula(collection, "總計", "2 + 2")?;
        assert!(notebook.collection_formulas(collection)?.is_empty());
        let edited = notebook.edit_canvas(id, &doc, super::super::canvas_edit::CanvasEdit::AddNode {
            kind: "text", content: "新內容", geometry: [0, 0, 100, 50],
        })?;
        assert!(notebook.remove_canvas(id, &doc).is_err());
        assert_eq!(notebook.read_canvas(id)?.as_str(), edited.as_str());
        notebook.remove_canvas(id, &edited)?;
        assert!(notebook.read_canvas(id).is_err());
        assert_ne!(notebook.save_canvas("再次匯入", &doc)?, id);
        Ok(())
    }
}
