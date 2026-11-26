use super::*;
use std::{fs, path::Path};

fn add_file(db: &mut Notebook, note: i64, dir: &Path, bytes: &[u8]) -> Result<i64> {
    let path = dir.join("同名.txt");
    fs::write(&path, bytes)?;
    db.import_attachment(note, &path)
}

fn payload(db: &Notebook, id: i64) -> Result<Vec<u8>> {
    Ok(db.connection.query_row("SELECT content FROM attachments WHERE id=?1", [id], |r| r.get(0))?)
}

#[test]
fn v31_text_history_migrates_without_clearing_live_properties_or_attachments() -> Result<()> {
    let dir = super::super::tests::data_dir();
    let result = (|| -> Result<()> {
        let path = dir.join("legacy.db");
        let mut db = Notebook::open(&path, false)?;
        let note = db.add(None, "舊版", "舊正文")?;
        db.set_property(note, "欄位", Some("應保留"))?;
        let file = add_file(&mut db, note, &dir, b"current original bytes")?;
        super::super::tests::remove_v31_schema(&db.connection)?;
        db.connection.execute("DELETE FROM note_revisions", [])?;
        db.connection.pragma_update(None, "user_version", 31)?;
        db.connection.execute("UPDATE notes SET title='目前',body='現行',version=version+1 WHERE id=?1", [note])?;
        drop(db);
        let db = Notebook::open(&path, false)?;
        let revision = db.note_revisions(note)?.remove(0);
        let preview = db.preview_note_revision(note, revision.id)?;
        assert!(preview.metadata.contains("只保存標題與正文"));
        assert!(!preview.metadata_changed);
        assert!(db.restore_note_revision(note, db.note(note)?.unwrap().version, revision.id, Some(preview.data_version))?);
        assert_eq!(db.note(note)?.unwrap().body, "舊正文");
        assert_eq!(db.properties(note)?, [("欄位".into(), "應保留".into())]);
        assert_eq!(payload(&db, file)?, b"current original bytes");
        assert_eq!(db.connection.pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))?, super::super::SCHEMA_VERSION);
        assert!(db.preview_note_revision(note, db.note_revisions(note)?[0].id)?.metadata.contains("同名.txt"));
        Ok(())
    })();
    let _ = fs::remove_dir_all(dir);
    result
}

#[test]
fn same_name_deleted_bytes_and_metadata_restore_after_reopen_and_backup() -> Result<()> {
    let dir = super::super::tests::data_dir();
    let result = (|| -> Result<()> {
        let path = dir.join("live.db");
        let mut db = Notebook::open(&path, false)?;
        let note = db.add(None, "兩個同名附件", "正文不變")?;
        let other = db.add(None, "另一篇", "")?;
        let first = add_file(&mut db, note, &dir, b"AAA")?;
        let second = add_file(&mut db, note, &dir, b"BBB")?;
        let elsewhere = add_file(&mut db, other, &dir, b"OTHER")?;
        db.set_property(note, "欄位", Some("舊值"))?;
        db.remove_attachment(first)?;
        let selected = db.note_revisions(note)?[0].id;
        let replacement = add_file(&mut db, note, &dir, b"CCC")?;
        db.set_property(note, "欄位", Some("現值"))?;
        db.set_property(note, "多的屬性", Some("現行"))?;
        drop(db);

        let db = Notebook::open(&path, false)?;
        let preview = db.preview_note_revision(note, selected)?;
        assert!(preview.metadata_changed);
        assert_eq!(preview.body, "正文不變");
        assert!(preview.metadata.contains("欄位 = 舊值"));
        assert!(preview.metadata.contains(&format!("同名.txt　3 位元組　#{first}")));
        assert!(preview.metadata.contains(&format!("同名.txt　3 位元組　#{second}")));
        let version = db.note(note)?.unwrap().version;
        assert!(db.restore_note_revision(note, version, selected, Some(preview.data_version))?);
        assert_eq!(db.note(note)?.unwrap().version, version);
        assert!(!db.restore_note_revision(note, version, selected, None)?);
        let backup = dir.join("backup.db");
        let isolated = dir.join("isolated.db");
        db.backup(&backup)?;
        drop(db);
        fs::copy(&backup, &isolated)?;
        let copy = Notebook::open(&isolated, false)?;
        assert_eq!(copy.properties(note)?, [("欄位".into(), "舊值".into())]);
        assert_eq!(copy.list_attachments(note)?.iter().map(|a| a.id).collect::<Vec<_>>(), [first,second]);
        assert_eq!(payload(&copy, first)?, b"AAA");
        assert_eq!(payload(&copy, second)?, b"BBB");
        assert_eq!(payload(&copy, elsewhere)?, b"OTHER");
        assert!(payload(&copy, replacement).is_err());
        let replaced = copy.note_revisions(note)?[0].id;
        assert!(copy.restore_note_revision(note, version, replaced, None)?);
        assert_eq!(payload(&copy, replacement)?, b"CCC");
        assert_eq!(Notebook::open(&path, false)?.properties(note)?, [("欄位".into(), "舊值".into())]);
        Ok(())
    })();
    let _ = fs::remove_dir_all(dir);
    result
}

#[test]
fn restore_rolls_back_after_late_file_failure_and_rejects_stale_review() -> Result<()> {
    let dir = super::super::tests::data_dir();
    let result = (|| -> Result<()> {
        let path = dir.join("atomic.db");
        let mut db = Notebook::open(&path, false)?;
        let note = db.add(None, "原名", "原文")?;
        let file = add_file(&mut db, note, &dir, b"original")?;
        db.set_property(note, "欄位", Some("舊值"))?;
        let version = db.save(note, db.note(note)?.unwrap().version, "改名", "改文")?;
        let selected = db.note_revisions(note)?[0].id;
        db.remove_attachment(file)?;
        let next = add_file(&mut db, note, &dir, b"retained")?;
        db.set_property(note, "欄位", Some("現值"))?;

        let preview = db.preview_note_revision(note, selected)?;
        let peer = Notebook::open(&path, false)?;
        peer.set_property(note, "欄位", Some("另一視窗"))?;
        assert!(db.restore_note_revision(note, version, selected, Some(preview.data_version))
            .unwrap_err().to_string().contains("預覽後已變更"));
        assert_eq!(db.note(note)?.unwrap().body, "改文");
        assert_eq!(payload(&db, next)?, b"retained");
        drop(peer);

        // Fail on the actual INSERT after text, properties and removal have run.
        db.connection.execute_batch(&format!(
            "CREATE TRIGGER history_test_abort BEFORE INSERT ON attachments
             WHEN NEW.id={file} BEGIN SELECT RAISE(ABORT,'restore blocked at attachment insert'); END;"
        ))?;
        let revisions = db.note_revisions(note)?.iter().map(|r| r.id).collect::<Vec<_>>();
        let error = db.restore_note_revision(note, version, selected, None).unwrap_err();
        assert!(error.to_string().contains("restore blocked at attachment insert"));
        assert_eq!(db.note(note)?.unwrap().row.title, "改名");
        assert_eq!(db.note(note)?.unwrap().body, "改文");
        assert_eq!(db.properties(note)?, [("欄位".into(), "另一視窗".into())]);
        assert_eq!(payload(&db, next)?, b"retained");
        assert!(payload(&db, file).is_err());
        assert_eq!(db.note_revisions(note)?.iter().map(|r| r.id).collect::<Vec<_>>(), revisions);
        Ok(())
    })();
    let _ = fs::remove_dir_all(dir);
    result
}

#[test]
fn restore_refuses_to_remove_a_pdf_used_by_a_current_annotation() -> Result<()> {
    let dir = super::super::tests::data_dir();
    let result = (|| -> Result<()> {
        let mut db = Notebook::open(&dir.join("annotated.db"), false)?;
        let note = db.add(None, "原名", "原文")?;
        let version = db.save(note, 1, "現名", "現文")?;
        let selected = db.note_revisions(note)?[0].id;
        let path = dir.join("source.pdf");
        fs::write(&path, b"%PDF-1.4\n%%EOF\n")?;
        let attachment = db.import_attachment(note, &path)?;
        let annotation = db.add_pdf_annotation(attachment, 1, "摘錄", "")?;
        let error = db.restore_note_revision(note, version, selected, None).unwrap_err();
        assert!(error.to_string().contains("PDF 標註"));
        assert_eq!(db.note(note)?.unwrap().body, "現文");
        assert_eq!(payload(&db, attachment)?, fs::read(path)?);
        let stored: i64 = db.connection.query_row(
            "SELECT attachment_id FROM pdf_annotations WHERE id=?1", [annotation], |r| r.get(0),
        )?;
        assert_eq!(stored, attachment);
        Ok(())
    })();
    let _ = fs::remove_dir_all(dir);
    result
}

#[test]
fn text_only_restore_preserves_pinned_excerpt_and_properties_across_backup() -> Result<()> {
    let dir=super::super::tests::data_dir();
    let path=dir.join("live.db");
    let backup=dir.join("saved.db");
    let mut db=Notebook::open(&path,false)?;
    let note=db.add(None,"初稿","早期正文")?;
    let version=db.save(note,1,"定稿","目前正文")?;
    let revision=db.note_revisions(note)?[0].id;
    let file=dir.join("鏡頭.pdf");
    let bytes=include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
    fs::write(&file,bytes)?;
    let attachment=db.import_attachment(note,&file)?;
    let mark=db.add_pdf_annotation(attachment,1,"底片","保留註解")?;
    let board=db.create_board("拍攝")?;
    let excerpt=db.create_pdf_excerpt_card(board,mark)?;
    let group=db.create_board_group(board,"出處")?;
    db.add_card_on(board,note)?;
    db.set_board_group(board,note,None,Some(group))?;
    db.set_board_group_source(board,group,None,Some(mark))?;
    let preview=db.preview_note_revision(note,revision)?;
    let blocked=preview.protected_files.join("\n");
    for label in ["鏡頭.pdf","PDF 標註","PDF 摘錄卡","白板分區釘選"] {assert!(blocked.contains(label),"{blocked}");}
    let writer=Notebook::open(&path,false)?;
    writer.set_property(note,"審稿",Some("另一視窗確認"))?;
    assert!(db.restore_revision_text(note,version,revision,preview.data_version).is_err());
    assert_eq!(db.note(note)?.unwrap().body,"目前正文");
    let current=db.preview_note_revision(note,revision)?;
    let before=db.note_revisions(note)?.len();
    assert!(db.restore_note_revision(note,version,revision,Some(current.data_version)).is_err());
    assert_eq!(db.note_revisions(note)?.len(),before);
    // Reject at the actual save step without an extra checkpoint or partial changes.
    db.connection.execute_batch("CREATE TEMP TRIGGER decline_text BEFORE UPDATE OF body ON notes
        BEGIN SELECT RAISE(ABORT,'synthetic rejected write'); END")?;
    assert!(db.restore_revision_text(note,version,revision,current.data_version).is_err());
    db.connection.execute_batch("DROP TRIGGER decline_text")?;
    assert_eq!(db.note_revisions(note)?.len(),before);
    assert!(db.restore_revision_text(note,version,revision,current.data_version)?);
    assert_eq!(db.note_revisions(note)?.len(),before+1);
    let after=db.preview_note_revision(note,revision)?;
    assert!(!db.restore_revision_text(note,version+1,revision,after.data_version)?);
    db.backup(&backup)?;
    drop(writer); drop(db);
    let isolated=dir.join("isolated.db");
    fs::copy(backup, &isolated)?;
    for path in [&path,&isolated] {
        let db=Notebook::open(path,false)?;
        let restored=db.note(note)?.unwrap();
        assert_eq!((restored.row.title.as_str(),restored.body.as_str()),("初稿","早期正文"));
        assert_eq!(db.properties(note)?,[("審稿".into(),"另一視窗確認".into())]);
        assert_eq!(payload(&db,attachment)?,bytes.as_slice());
        assert_eq!(db.pdf_excerpt_source(excerpt)?.unwrap().attachment_id,attachment);
        assert_eq!(db.live_board_group_source(board,group,mark)?.unwrap().attachment_id,attachment);
        assert_eq!(db.note_revision(note,db.note_revisions(note)?[0].id)?.1,"目前正文");
    }
    fs::remove_dir_all(dir)?;
    Ok(())
}

#[test]
fn protected_base_source_survives_history_and_backup_but_other_receipt_files_remain_editable() -> Result<()> {
    use crate::archive::ImportedPage;
    let dir = super::super::tests::data_dir();
    let live = dir.join("live.db");
    let backup = dir.join("backup.db");
    let mut db = Notebook::open(&live,false)?;
    let root = db.import_vault("合成",vec![ImportedPage {
        parts:vec!["A.md".into()], title:"A".into(), body:"正文".into(),
    }])?;
    let raw = b"views:\n  - type: table\n    name: Pin\n    order: [file.path]\n";
    let (receipt,_) = db.import_obsidian_base(root,"Shipping.base",raw.to_vec())?;
    let original = db.list_attachments(receipt)?[0].id;
    let doc = crate::canvas::parse(r##"{"nodes":[{"id":"p","type":"file","file":"Shipping.base","subpath":"#Pin","x":0,"y":0,"width":232,"height":114}]}"##.to_owned())?;
    let canvas = db.save_canvas("合成圖",&doc)?;
    db.set_canvas_vault(canvas,Some(root))?;
    db.bind_base_card(canvas,&doc,root,"p",receipt,None)?;
    let start = db.note_revisions(receipt)?.len();
    let other = Notebook::open(&live,false)?;
    assert!(other.remove_attachment(original).unwrap_err().to_string().contains("Bases 來源收據"));
    assert_eq!(db.note_revisions(receipt)?.len(),start);
    let extra = add_file(&mut db,receipt,&dir,b"independent")?;
    let revision = db.note_revisions(receipt)?[0].id;
    let preview = db.preview_note_revision(receipt,revision)?;
    assert!(!preview.base_source_blocked);
    let version = db.note(receipt)?.unwrap().version;
    assert!(db.restore_note_revision(receipt,version,revision,Some(preview.data_version))?);
    assert_eq!(db.list_attachments(receipt)?.len(),1);
    assert!(payload(&db,extra).is_err());
    // Model an imported old snapshot with different source bytes. Refusing this
    // operation must preserve the current note and all of its live source links.
    db.connection.execute("UPDATE note_revision_attachments SET content=?1 WHERE revision_id=?2 AND original_id=?3",
        params![b"historical different source".to_vec(),revision,original])?;
    let preview=db.preview_note_revision(receipt,revision)?;
    assert!(preview.base_source_blocked);
    let latest=db.note(receipt)?.unwrap().version;
    assert!(db.restore_note_revision(receipt,latest,revision,Some(preview.data_version)).is_err());
    db.backup(&backup)?;
    drop(other);drop(db);
    for path in [&live,&backup] {
        let db=Notebook::open(path,false)?;
        assert_eq!(payload(&db,original)?,raw);
        assert_eq!(db.base_path_receipt(canvas,root,"Shipping.base")?,Some(receipt));
        assert_eq!(db.resolve_base_card(canvas,&doc,root,"p")?.3.name,"Pin");
        assert_eq!(db.note(receipt)?.unwrap().version,latest);
    }
    fs::remove_dir_all(dir)?;
    Ok(())
}
