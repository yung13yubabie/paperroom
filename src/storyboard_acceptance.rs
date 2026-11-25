use super::{Notebook, Result, View};
use super::pdf_regions::PdfRegion;
use crate::{canvas, markdown};
use std::fs;

const CANVAS: &[u8] = include_bytes!("../tests/fixtures/storyboard-zh/分鏡.canvas");
const PDF: &[u8] = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
const CSV: &[u8] = include_bytes!("../tests/fixtures/storyboard-zh/關聯網址.csv");

#[test]
fn storyboard_round_trip_preserves_link_scope_regions_and_live_rollup() -> Result<()> {
    let root = std::env::temp_dir().join(format!("paperroom-storyboard-{}", std::process::id()));
    fs::create_dir(&root)?;
    let vault = root.join("合成筆記庫");
    fs::create_dir_all(vault.join("分鏡"))?;
    fs::create_dir_all(vault.join("場記"))?;
    fs::write(vault.join("分鏡/鏡頭.md"), include_bytes!("../tests/fixtures/storyboard-zh/Vault/分鏡/鏡頭.md"))?;
    fs::write(vault.join("場記/鏡頭.md"), include_bytes!("../tests/fixtures/storyboard-zh/Vault/場記/鏡頭.md"))?;
    fs::write(vault.join("分鏡/清單.md"), include_bytes!("../tests/fixtures/storyboard-zh/Vault/分鏡/清單.md"))?;
    let pdf = root.join("剪輯範圍.pdf");
    fs::write(&pdf, PDF)?;
    let canvas_path = root.join("分鏡.canvas");
    fs::write(&canvas_path, CANVAS)?;
    let live_path = root.join("live.sqlite3");
    let mut notebook = Notebook::open(&live_path, false)?;
    let scope = notebook.import_vault("合成筆記庫", crate::vault::read(&vault)?.pages)?;
    let a = only_note(&notebook, "分鏡專用青色逆光")?;
    let b = only_note(&notebook, "場記專用琥珀色輪廓光")?;
    let list = only_note(&notebook, "同名未限定的")?;
    assert_ne!(a, b);
    assert_eq!(notebook.note(a)?.unwrap().row.title, "鏡頭");
    assert_eq!(notebook.note(b)?.unwrap().row.title, "鏡頭");
    let refs = notebook.vault_note_links(&notebook.note(list)?.unwrap())?;
    assert_eq!(refs.len(), 1);
    assert_eq!((refs[0].note_id, refs[0].label.as_str(), refs[0].anchor.as_deref()),
               (b, "導演選用版", Some("#補拍焦段")));
    let b_note = notebook.note(b)?.unwrap();
    let section = markdown::heading_section(&b_note.body, "#補拍焦段").unwrap();
    assert!(section.contains("場記專用琥珀色輪廓光"));
    assert!(!section.contains("收音"));
    assert!(markdown::heading_section("## 補拍焦段\n舊\n## 補拍焦段\n新", "#補拍焦段").is_none());

    let original = canvas::read(&canvas_path)?;
    let id = notebook.save_canvas("分鏡", &original)?;
    notebook.set_canvas_vault(id, Some(scope))?;
    check_canvas(&notebook, id, a, b)?;

    let att = notebook.import_attachment(list, &pdf)?;
    let text = notebook.add_pdf_annotation(att, 1, "selection", "純文字摘錄")?;
    let region_a = PdfRegion { x: 0.08, y: 0.05, width: 0.50, height: 0.12 };
    let region_b = PdfRegion { x: 0.42, y: 0.58, width: 0.47, height: 0.10 };
    let box_a = notebook.add_pdf_annotation_with_region(att, 1, "selection", "頁一矩形", region_a)?;
    let box_b = notebook.add_pdf_annotation_with_region(att, 2, "selection", "頁二矩形", region_b)?;
    assert!(notebook.pdf_region(text)?.is_none());

    notebook.import_csv("合成 Notion CSV", CSV)?;
    let csv_note = only_note(&notebook, "外部 CSV 列")?;
    assert!(notebook.properties(csv_note)?.iter().any(|(key, value)| key == "候選連結" && value == "https://example.invalid/notes/shot"));
    assert!(notebook.relations(csv_note)?.is_empty());
    assert_eq!(notebook.rollup_count(list, "時長來源")?, 0);
    notebook.compare_set_property(a, "時長", None, Some("5"))?;
    notebook.compare_set_property(b, "時長", None, Some("7"))?;
    notebook.set_relation(list, "時長來源", a, true)?;
    notebook.set_relation(list, "時長來源", b, true)?;
    let first = root.join("12.sqlite3");
    let second = root.join("18.sqlite3");
    assert_eq!(notebook.rollup_sum(list, "時長來源", "時長")?, 12.0);
    notebook.backup(&first)?;
    let writer = Notebook::open(&live_path, false)?;
    writer.compare_set_property(b, "時長", Some("7"), Some("13"))?;
    writer.save(b, b_note.version, "更名仍在場記", &format!("{}\n補拍已確認。\n", b_note.body))?;
    assert_eq!(notebook.rollup_count(list, "時長來源")?, 2);
    assert_eq!(notebook.rollup_sum(list, "時長來源", "時長")?, 18.0);
    check_canvas(&notebook, id, a, b)?;
    notebook.backup(&second)?;
    drop(writer);
    drop(notebook);

    let live = Notebook::open(&live_path, false)?;
    assert_eq!(live.rollup_sum(list, "時長來源", "時長")?, 18.0);
    assert!(live.note(b)?.unwrap().body.contains("補拍已確認"));
    check_canvas(&live, id, a, b)?;
    drop(live);
    for (path, total) in [(&first, 12.0), (&second, 18.0)] {
        let saved = Notebook::open(path, false)?;
        let integrity: String = saved.connection.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
        assert_eq!(integrity, "ok");
        assert_eq!(saved.rollup_sum(list, "時長來源", "時長")?, total);
        assert_eq!(saved.rollup_count(list, "時長來源")?, 2);
        assert_eq!(saved.canvas_vault(id)?, Some(scope));
        check_canvas(&saved, id, a, b)?;
        assert_eq!(saved.pdf_region(box_a)?, Some(region_a));
        assert_eq!(saved.pdf_region(box_b)?, Some(region_b));
        assert!(saved.pdf_region(text)?.is_none());
        let entries = saved.list_pdf_annotations(att)?;
        assert!(entries.iter().any(|entry| entry.id == box_a && entry.page == 1 && entry.quote == "selection"));
        assert!(entries.iter().any(|entry| entry.id == box_b && entry.page == 2 && entry.quote == "selection"));
        let dir = root.join(total.to_string());
        fs::create_dir(&dir)?;
        let copy = dir.join("源.pdf");
        let exported = dir.join("分鏡.canvas");
        saved.save_attachment_copy(att, &copy)?;
        canvas::export(&exported, &saved.read_canvas(id)?)?;
        assert_eq!(fs::read(copy)?, PDF);
        assert_eq!(fs::read(exported)?, CANVAS);
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

fn only_note(db: &Notebook, query: &str) -> Result<i64> {
    let found = db.rows(View::All, query)?;
    assert_eq!(found.len(), 1, "expected a unique note for {query}");
    Ok(found[0].id)
}

fn check_canvas(db: &Notebook, id: i64, a: i64, b: i64) -> Result<()> {
    let links = db.canvas_file_links(id)?;
    assert_eq!(links.len(), 3);
    assert_eq!((links[0].node_id.as_str(), links[0].note_id, links[0].label.as_deref(), links[0].subpath.as_deref()),
               ("script-a", Some(a), Some("初剪"), Some("#補拍焦段")));
    assert_eq!((links[1].node_id.as_str(), links[1].note_id, links[1].subpath.as_deref()),
               ("script-b", Some(a), Some("#器材")));
    assert_eq!((links[2].node_id.as_str(), links[2].note_id), ("continuity", Some(b)));
    let raw = db.read_canvas(id)?;
    assert_eq!(raw.as_str().as_bytes(), CANVAS);
    assert_eq!(raw.json()["nodes"][1]["x"], -100);
    assert_eq!(raw.json()["nodes"][2]["x"], 220);
    assert_eq!(raw.json()["edges"][0]["label"], "重新安排");
    assert_eq!(raw.json()["edges"][0]["toEnd"], "arrow");
    Ok(())
}
