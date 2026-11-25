//! Exercise #9 through #12 on one workspace upgraded from the v30 schema.
use super::{
    Notebook, Result, canvas_bundle::LocalCanvasBundle, collection_field_rules::CollectionFieldRule,
    collections::CollectionEntry, typed_properties::PropertyType,
};
use crate::archive::notion_archive_with_media;
use rusqlite::Connection;
use serde_json::Value;
use std::{fs, io::Write, path::Path};
use zip::{ZipWriter, write::SimpleFileOptions};

const PROJECT: &str = "專案 11111111111111111111111111111111";
const ASSET: &str = "附件/畫面.png";
const NESTED_CANVAS: &str = "Boards/VaultRestoreScene.canvas";

fn assert_links(db: &Notebook, root: i64) -> Result<()> {
    let id_at = |member: &str| -> Result<i64> {
        Ok(db.connection.query_row(
            "SELECT note_id FROM notion_page_paths WHERE import_root=?1 AND relative_path=?2",
            (root, format!("{PROJECT}/{member}")), |row| row.get(0),
        )?)
    };
    let source = db.note(id_at("入口.md")?)?.unwrap();
    let project = db.connection.query_row(
        "SELECT note_id FROM notion_page_paths WHERE import_root=?1 AND relative_path=?2",
        (root, format!("{PROJECT}.md")), |row| row.get(0),
    )?;
    assert_eq!(source.row.parent, Some(project));
    let links = db.vault_note_links(&source)?;
    assert_eq!(links.len(), 2);
    assert_ne!(links[0].note_id, links[1].note_id);
    for (link, (label, member, expected)) in links.iter().zip([
        ("到甲", "甲/鏡頭.md", "甲子頁專屬正文。"),
        ("到乙", "乙/鏡頭.md", "乙子頁專屬正文。"),
    ]) {
        assert_eq!(link.label, label);
        assert_eq!(link.note_id, id_at(member)?);
        assert_eq!(link.notion_path, Some(format!("{PROJECT}/{member}")));
        let note = db.note(link.note_id)?.unwrap();
        assert_eq!(note.row.title, "同名鏡頭");
        assert!(note.body.contains(expected));
    }
    Ok(())
}

fn csv_source(db: &Notebook, root: i64, filename: &str, bytes: &[u8]) -> Result<(i64, i64)> {
    let (note, attachment, actual): (i64, i64, Vec<u8>) = db.connection.query_row(
        "SELECT a.note_id,a.id,a.content FROM attachments a
         JOIN notes n ON n.id=a.note_id JOIN notes f ON f.id=n.parent
         WHERE f.parent=?1 AND a.name=?2",
        (root, filename), |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(actual, bytes);
    Ok((note, attachment))
}

fn materialize_fixture(dir: &Path, fixture: &Value) -> Result<()> {
    let mut zip = ZipWriter::new(fs::File::create_new(dir.join("notion.zip"))?);
    for (path, source) in fixture["notion"].as_object().unwrap() {
        zip.start_file(path, SimpleFileOptions::default())?;
        zip.write_all(source.as_str().unwrap().as_bytes())?;
    }
    zip.finish()?;
    fs::create_dir(dir.join("附件"))?;
    image::RgbImage::from_pixel(16, 16, image::Rgb([37, 146, 179])).save(dir.join(ASSET))?;
    fs::write(dir.join("共同畫布.canvas"), fixture["canvas"].to_string())?;
    let vault = dir.join("Vault");
    for (member, colors) in fixture["vault_asset_colors"].as_object().unwrap() {
        for (scope, color) in [(&vault, "root"), (&vault.join("Boards"), "decoy")] {
            let target = scope.join(member);
            fs::create_dir_all(target.parent().unwrap())?;
            let rgb = std::array::from_fn(|index| colors[color][index].as_u64().unwrap() as u8);
            image::RgbImage::from_pixel(16, 16, image::Rgb(rgb)).save(target)?;
        }
    }
    fs::write(vault.join(NESTED_CANVAS), fixture["vault_canvas"].to_string())?;
    Ok(())
}

#[test]
fn csv_and_exact_links_coexist_with_canvas_and_legacy_rules_through_isolated_restore() -> Result<()> {
    let dir = super::tests::data_dir();
    let result = (|| -> Result<()> {
        let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/notion-canvas-integration.json"))?;
        materialize_fixture(&dir, &fixture)?;
        let workspace = dir.join("workspace.sqlite3");
        let archive = dir.join("notion.zip");
        let original = fs::read(&archive)?;
        let backup = dir.join("backup.sqlite3");
        let restore = dir.join("isolated.sqlite3");
        let raw_canvas = fs::read_to_string(dir.join("共同畫布.canvas"))?;
        let png = fs::read(dir.join(ASSET))?;
        let mut legacy = Notebook::open(&workspace, false)?;
        let marker = legacy.add(None, "既有筆記", "保留舊工作區正文")?;
        let existing = legacy.create_collection("既有集合")?;
        legacy.compare_set_property_type("工時", None, Some(PropertyType::Number))?;
        let rule = CollectionFieldRule { key: "工時".into(), kind: PropertyType::Number, required: true, default: Some("8".into()) };
        legacy.save_collection_field_rule(existing, &rule, None)?;
        legacy.add_to_collection(existing, marker, "待處理")?;
        super::tests::remove_v30_schema(&legacy.connection)?;
        legacy.connection.pragma_update(None, "user_version", 30)?;
        drop(legacy);
        {
            let old = Connection::open(&workspace)?;
            assert_eq!(old.pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))?, 30);
            assert!(!old.prepare("SELECT 1 FROM sqlite_schema WHERE name='notion_page_paths'")?.exists([])?);
        }
        let mut db = Notebook::open(&workspace, false)?;
        assert_eq!(db.connection.pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))?, 31);
        assert_eq!(db.collection_field_rules(existing)?, [rule.clone()]);
        assert_eq!(db.note(marker)?.unwrap().body, "保留舊工作區正文");
        assert_eq!(db.connection.query_row("SELECT count(*) FROM notion_page_paths", [], |row| row.get::<_, i64>(0))?, 0);

        let (pages, media, csv) = notion_archive_with_media(&archive)?;
        let root = db.import_notion_with_media("合成匯入", pages.pages, media, csv)?;
        assert_links(&db, root)?;
        let accepted = "BB-valid.csv";
        let rejected = "AA-conflict.csv";
        let csv_bytes = |name| fixture["notion"][format!("{PROJECT}/{name}")].as_str().unwrap().as_bytes();
        let (source, attachment) = csv_source(&db, root, accepted, csv_bytes(accepted))?;
        let (collection, num_rows) = db.create_collection_from_notion_csv(source, attachment, "聯合驗收集合")?;
        assert_eq!(num_rows, 2);
        let row: i64 = db.connection.query_row(
            "SELECT n.id FROM collection_notes m JOIN notes n ON n.id=m.note_id WHERE m.collection_id=?1 AND n.title='追蹤列甲'",
            [collection], |row| row.get(0),
        )?;
        db.set_property(row, "備註", Some("修改後欄值"))?;
        assert_links(&db, root)?;

        let (source, attachment) = csv_source(&db, root, rejected, csv_bytes(rejected))?;
        let changes = db.connection.total_changes();
        let error = db.create_collection_from_notion_csv(source, attachment, "禁止建立").unwrap_err();
        assert!(error.to_string().contains("「工時」已在工作區宣告為非文字屬性"));
        assert_eq!(db.connection.total_changes(), changes);
        assert_eq!(db.collections()?.len(), 2);

        let (canvas, _) = LocalCanvasBundle::read(&dir.join("共同畫布.canvas"))?.save(&db)?;
        drop(db);
        let db = Notebook::open(&workspace, false)?;
        let canvas_zip = dir.join("canvas.zip");
        assert_eq!(db.read_canvas(canvas)?.as_str(), raw_canvas);
        db.export_canvas_bundle(canvas, &canvas_zip)?;
        let copied_canvas = db.import_canvas_bundle(&canvas_zip)?.id;
        assert_ne!(canvas, copied_canvas);
        let vault = dir.join("Vault");
        let nested = vault.join(NESTED_CANVAS);
        let vault_canvas = LocalCanvasBundle::read_vault(&nested, &vault)?.save(&db)?.0;
        db.backup(&backup)?;
        drop(db);
        let before = fs::read(&workspace)?;
        let backup_bytes = fs::read(&backup)?;
        fs::copy(&backup, &restore)?;
        let detached_vault = dir.join("DetachedVault");
        fs::rename(&vault, &detached_vault)?;
        assert!(!nested.exists());
        let restored = Notebook::open(&restore, false)?;
        assert_links(&restored, root)?;
        assert!(restored.properties(row)?.contains(&("備註".into(), "修改後欄值".into())));
        assert!(restored.properties(row)?.contains(&("編號".into(), "00042".into())));
        assert_eq!(restored.collection_field_rules(existing)?, [rule]);
        let folder = restored.note(row)?.unwrap().row.parent.unwrap();
        let parent = restored.note(folder)?.unwrap().row.parent;
        assert_eq!(parent, Some(csv_source(&restored, root, accepted, csv_bytes(accepted))?.0));
        assert_eq!(restored.connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?, "ok");
        assert!(!restored.connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
        for id in [canvas, copied_canvas] {
            assert_eq!(restored.read_canvas(id)?.as_str(), raw_canvas);
            let file = dir.join(format!("picture-{id}.png"));
            restored.save_canvas_bundle_asset(id, ASSET, &file)?;
            assert_eq!(fs::read(file)?, png);
        }
        assert_eq!(restored.read_canvas(vault_canvas)?.as_str(), fs::read_to_string(detached_vault.join(NESTED_CANVAS))?);
        for member in fixture["vault_asset_colors"].as_object().unwrap().keys() {
            let file = dir.join(Path::new(member).file_name().unwrap());
            restored.save_canvas_bundle_asset(vault_canvas, member, &file)?;
            let kept = fs::read(file)?;
            assert_eq!(kept, fs::read(detached_vault.join(member))?);
            assert_ne!(kept, fs::read(detached_vault.join("Boards").join(member))?);
        }
        let new_row = restored.add_collection_entry(existing, CollectionEntry::New("隔離還原追加"), "待處理", None)?;
        assert_eq!(restored.properties(new_row)?, [("工時".into(), "8".into())]);
        drop(restored);
        assert_eq!(fs::read(&workspace)?, before);
        assert_eq!(fs::read(&backup)?, backup_bytes);
        assert_eq!(fs::read(&archive)?, original);
        Ok(())
    })();
    let _ = fs::remove_dir_all(dir);
    result
}
