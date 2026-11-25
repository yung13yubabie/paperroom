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
        assert!(error.to_string().contains("仍被標註"));
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
