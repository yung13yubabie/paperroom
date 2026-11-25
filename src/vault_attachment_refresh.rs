//! Refresh only known relative paths whose Markdown and attachment copies still agree.
use super::{MediaPaths, Result, Vault, file_kind, read_bytes, referenced_paths, resolve, safe};
use super::super::{Notebook, attachments::MAX_STORAGE_BYTES};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{collections::HashMap, path::{Path, PathBuf}};

#[derive(Debug, PartialEq, Eq)]
struct CopyState {
    id: i64,
    note_path: String,
    version: i64,
    protected: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Local {
    root: i64,
    relative: String,
    contents: Vec<u8>,
    copies: Vec<CopyState>,
}

#[derive(PartialEq, Eq)]
pub(crate) struct AttachmentRefresh {
    note: i64,
    attachment: i64,
    directory: PathBuf,
    local: Local,
    incoming: Vec<u8>,
}

impl AttachmentRefresh {
    pub(crate) fn count(&self) -> usize { self.local.copies.len() }

    pub(crate) fn actionable(&self) -> bool {
        self.local.contents != self.incoming && !self.local.copies.iter().any(|copy| copy.protected)
    }

    pub(crate) fn report(&self) -> String {
        let state = if self.local.contents == self.incoming {
            "紙間與磁碟的附件位元相同。"
        } else if self.local.copies.iter().any(|copy| copy.protected) {
            "原 PDF 有標註或摘錄來源。為保留原選區，這次維持原檔內容及來源連結。"
        } else {
            "已驗證來源 Markdown 與紙間的共同版本相同。可更新此批次的所有附件副本，原附件 ID 會保留。"
        };
        let mut text = format!(
            "來源資料夾：{}\r\n相對路徑：{}\r\n紙間 {} 位元組；磁碟 {} 位元組。\r\n{state}\r\n共 {} 份，來源筆記：",
            self.directory.display(), self.local.relative, self.local.contents.len(),
            self.incoming.len(), self.count(),
        );
        for copy in self.local.copies.iter().take(100) {
            text.push_str("\r\n");
            text.push_str(&copy.note_path);
        }
        if self.count() > 100 {
            text.push_str(&format!("\r\n另有 {} 份。", self.count() - 100));
        }
        text
    }
}

fn local_state(
    conn: &Connection,
    note: i64,
    attachment: i64,
    source: Option<&Vault>,
) -> Result<Local> {
    let (root, relative, kind, contents): (i64, String, String, Vec<u8>) = conn.query_row(
        "SELECT p.import_root,o.relative_path,a.kind,a.content FROM attachments a
         JOIN attachment_origins o ON o.attachment_id=a.id
         JOIN vault_paths p ON p.note_id=a.note_id
         JOIN notes n ON n.id=a.note_id JOIN notes r ON r.id=p.import_root
         WHERE a.note_id=?1 AND a.id=?2 AND n.trash_root IS NULL AND r.trash_root IS NULL",
        params![note, attachment],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional()?.ok_or("找不到這個仍使用中的 Vault 來源附件。")?;
    if !relative.split('/').all(safe) || file_kind(Path::new(&relative))? != kind {
        return Err("附件的來源路徑或格式無法驗證。".into());
    }

    let disk = source.map(|vault| {
        let pages: HashMap<_, _> = vault.pages.iter().map(|p| (p.parts.join("/"), p)).collect();
        (MediaPaths::new(vault), pages)
    });
    if disk.as_ref().is_some_and(|(media, _)| !media.paths.contains(relative.as_str())) {
        return Err("磁碟未讀到完全相符的附件路徑。請檢查資料夾、檔名大小寫與來源位置。".into());
    }
    let mut stmt = conn.prepare(
        "SELECT a.id,p.relative_path,n.version,
                n.trash_root IS NULL AND v.note_id IS NOT NULL AND coalesce(b.body=n.body,0)
                  AND a.kind=?3 AND a.content=?4,
                EXISTS(SELECT 1 FROM pdf_annotations x WHERE x.attachment_id=a.id)
                OR EXISTS(SELECT 1 FROM pdf_excerpt_origins x WHERE x.attachment_id=a.id),
                n.body
         FROM attachment_origins o JOIN attachments a ON a.id=o.attachment_id
         JOIN vault_paths p ON p.note_id=a.note_id JOIN notes n ON n.id=a.note_id
         LEFT JOIN vault_sync_base b ON b.note_id=n.id LEFT JOIN vault_push_verified v ON v.note_id=n.id
         WHERE p.import_root=?1 AND o.relative_path=?2 ORDER BY p.relative_path,a.id"
    )?;
    let mut rows = stmt.query(params![root, relative, kind, contents])?;
    let mut copies = Vec::new();
    while let Some(row) = rows.next()? {
        let path: String = row.get(1)?;
        if !row.get::<_, bool>(3)? {
            return Err(format!("{path} 的附件副本或 Markdown 共同版本不一致，這次維持工作區原狀。").into());
        }
        if let Some((media, pages)) = &disk {
            let body = row.get_ref(5)?.as_str()?;
            let verified = pages.get(&path).is_some_and(|page| {
                page.body == body && referenced_paths(&page.body).any(|(link, wiki)| {
                    resolve(&page.parts, &link, wiki, &media.paths, &media.names).as_deref() == Some(relative.as_str())
                })
            });
            if !verified {
                return Err(format!("{path} 的磁碟 Markdown 與共同版本不同，或附件引用不再明確。請確認正確 Vault 來源。").into());
            }
        }
        copies.push(CopyState { id: row.get(0)?, note_path: path, version: row.get(2)?, protected: row.get(4)? });
    }
    Ok(Local { root, relative, contents, copies })
}

impl Notebook {
    pub(crate) fn vault_attachment_choices(&self, note: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.connection.prepare(
            "SELECT a.id,o.relative_path FROM attachments a
             JOIN attachment_origins o ON o.attachment_id=a.id
             JOIN vault_paths p ON p.note_id=a.note_id
             JOIN notes n ON n.id=a.note_id JOIN notes r ON r.id=p.import_root
             WHERE a.note_id=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL
             ORDER BY o.relative_path,a.id",
        )?;
        Ok(stmt.query_map([note], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn preview_vault_attachment(
        &self,
        note: i64,
        attachment: i64,
        directory: &Path,
    ) -> Result<AttachmentRefresh> {
        let source = crate::vault::scan_for_sync(directory)?;
        let tx = self.connection.unchecked_transaction()?;
        let local = local_state(&tx, note, attachment, Some(&source))?;
        tx.commit()?;
        let incoming = read_bytes(directory, &local.relative, MAX_STORAGE_BYTES as usize)?;
        Ok(AttachmentRefresh { note, attachment, directory: directory.into(), local, incoming })
    }

    pub(crate) fn apply_vault_attachment(
        &mut self,
        preview: &AttachmentRefresh,
        backup: &Path,
    ) -> Result<usize> {
        if !preview.actionable() {
            return Err("這個附件沒有可套用的更新。".into());
        }
        let current = self.preview_vault_attachment(preview.note, preview.attachment, &preview.directory)?;
        if current != *preview {
            return Err("磁碟或紙間內容已變更，請重新預覽。".into());
        }
        drop(current);
        self.backup(backup)?;
        self.commit_attachment_refresh(preview)
            .map_err(|error| format!("{error}　備份已保留：{}", backup.display()).into())
    }

    fn commit_attachment_refresh(&mut self, preview: &AttachmentRefresh) -> Result<usize> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let local = local_state(&tx, preview.note, preview.attachment, None)?;
        let disk = read_bytes(&preview.directory, &local.relative, MAX_STORAGE_BYTES as usize)?;
        if local != preview.local || disk != preview.incoming {
            return Err("磁碟或紙間內容已變更，請重新預覽。".into());
        }
        let used: i64 = tx.query_row("SELECT coalesce(sum(length(content)),0) FROM attachments", [], |r| r.get(0))?;
        let increase = (disk.len() as i64 - local.contents.len() as i64) * local.copies.len() as i64;
        if used + increase > MAX_STORAGE_BYTES {
            return Err("更新會超過工作區附件總容量 128 MiB。".into());
        }
        {
            let mut update = tx.prepare("UPDATE attachments SET content=?2 WHERE id=?1")?;
            for copy in &local.copies {
                update.execute(params![copy.id, disk])?;
            }
        }
        tx.commit()?;
        Ok(local.copies.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn import(db: &mut Notebook, path: &Path) -> Result<Vec<i64>> {
        let vault = crate::vault::read(path)?;
        let media = super::super::plan(path, &vault)?;
        let root = db.import_vault_with_media("測試 Vault", vault.pages, media)?;
        Ok(db.search_vault_sources(root, "", "", "", super::super::super::vault_search::TagMatch::Exact)?
            .into_iter().map(|(note, _)| note).collect())
    }

    fn attachment(db: &Notebook, note: i64) -> Result<i64> {
        Ok(db.vault_attachment_choices(note)?[0].0)
    }

    fn blob(db: &Notebook, id: i64) -> Result<Vec<u8>> {
        Ok(db.connection.query_row("SELECT content FROM attachments WHERE id=?1", [id], |r| r.get(0))?)
    }

    #[test]
    fn batch_refresh_keeps_attachment_ids_preserves_originals_and_recovers_old_bytes() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("合成 Vault");
        let relative = "媒體/場記.txt";
        let path = dir.join("live.sqlite");
        let backup = dir.join("backup.sqlite");
        let body = "[日誌](媒體/場記.txt)";
        fs::create_dir_all(vault.join("媒體"))?;
        fs::write(vault.join("A.md"), body)?;
        fs::write(vault.join("B.md"), "![[場記.txt]]")?;
        fs::write(vault.join(relative), "原本")?;

        let mut db = Notebook::open(&path, false)?;
        let notes = import(&mut db, &vault)?;
        let same_path_other_batch = import(&mut db, &vault)?;
        let ids = notes.iter().map(|&note| attachment(&db, note)).collect::<Result<Vec<_>>>()?;
        let after = attachment(&db, same_path_other_batch[0])?;
        assert!(!db.preview_vault_attachment(notes[0], ids[0], &vault)?.actionable());
        fs::write(vault.join(relative), "更新")?;
        let preview = db.preview_vault_attachment(notes[0], ids[0], &vault)?;
        assert!(preview.actionable());
        assert_eq!(preview.count(), 2);
        assert!(preview.report().contains("B.md"));
        assert_eq!(db.apply_vault_attachment(&preview, &backup)?, 2);
        let exported = dir.join("新位元.txt");
        db.save_attachment_copy(ids[0], &exported)?;
        assert_eq!(fs::read(&exported)?, "更新".as_bytes());
        assert_eq!(db.note(notes[0])?.unwrap().body, body);
        assert!(!db.preview_vault_attachment(notes[0], ids[0], &vault)?.actionable());
        drop(db);

        let db = Notebook::open(&path, false)?;
        let old = Notebook::open(&backup, false)?;
        for (note, id) in notes.into_iter().zip(ids) {
            assert_eq!(attachment(&db, note)?, id);
            assert_eq!(db.attachment_origin(id)?.as_deref(), Some(relative));
            assert_eq!(blob(&db, id)?, "更新".as_bytes());
            assert_eq!(blob(&old, id)?, "原本".as_bytes());
        }
        assert_eq!(blob(&db, after)?, "原本".as_bytes());
        assert_eq!(fs::read(vault.join(relative))?, "更新".as_bytes());
        assert_eq!(fs::read_to_string(vault.join("A.md"))?, body);
        assert_eq!(fs::read_dir(vault.join("媒體"))?.count(), 1);
        drop((db, old));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn stale_disk_conflicts_and_failed_backup_leave_both_sides_intact() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let path = dir.join("live.sqlite");
        let backup = dir.join("backup.sqlite");
        let body = "![[備忘.txt]]";
        let source = vault.join("備忘.txt");
        fs::create_dir(&vault)?;
        fs::write(vault.join("來源.md"), body)?;
        fs::write(&source, "初始")?;
        let mut db = Notebook::open(&path, false)?;
        let note = import(&mut db, &vault)?[0];
        let id = attachment(&db, note)?;

        fs::write(&source, "初稿")?;
        let preview = db.preview_vault_attachment(note, id, &vault)?;
        fs::write(&source, "改稿")?;
        assert!(db.apply_vault_attachment(&preview, &backup).is_err());
        assert!(!backup.exists());
        assert_eq!(blob(&db, id)?, "初始".as_bytes());

        let preview = db.preview_vault_attachment(note, id, &vault)?;
        fs::write(&backup, "existing backup")?;
        assert!(db.apply_vault_attachment(&preview, &backup).is_err());
        assert_eq!(fs::read_to_string(&backup)?, "existing backup");
        assert_eq!(blob(&db, id)?, "初始".as_bytes());
        let n = db.note(note)?.unwrap();
        db.save(note, n.version, &n.row.title, "另一端已改")?;
        assert!(db.apply_vault_attachment(&preview, &dir.join("unused.sqlite")).is_err());
        db.save(note, n.version + 1, &n.row.title, body)?;
        fs::create_dir(vault.join("同名"))?;
        fs::write(vault.join("同名/備忘.txt"), "同名來源")?;
        assert!(db.preview_vault_attachment(note, id, &vault).is_err());
        fs::remove_file(vault.join("同名/備忘.txt"))?;
        fs::rename(&source, vault.join("已改名.txt"))?;
        assert!(db.preview_vault_attachment(note, id, &vault).is_err());
        assert!(!dir.join("unused.sqlite").exists());
        assert_eq!(blob(&db, id)?, "初始".as_bytes());
        assert_eq!(fs::read(vault.join("已改名.txt"))?, "改稿".as_bytes());
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn changed_pdf_is_blocked_by_geometry_or_a_recoverable_excerpt_origin() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let path = dir.join("live.sqlite");
        let backup = dir.join("unused.sqlite");
        let bytes = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
        let source = vault.join("原稿.pdf");
        fs::create_dir(&vault)?;
        fs::write(vault.join("來源.md"), "[原稿](原稿.pdf)")?;
        fs::write(&source, bytes)?;
        let mut db = Notebook::open(&path, false)?;
        let note = import(&mut db, &vault)?[0];
        let id = attachment(&db, note)?;
        let board = db.create_board("來源白板")?;
        let new_bytes = [&bytes[..], b"\n% changed\n"].concat();
        fs::write(&source, &new_bytes)?;

        let stale = db.preview_vault_attachment(note, id, &vault)?;
        assert!(stale.actionable());
        let region = crate::store::PdfRegion { x: 0.1, y: 0.25, width: 0.3, height: 0.15 };
        let annotation = db.add_pdf_annotation_with_region(id, 1, "原本文字", "", region)?;
        let card = db.create_pdf_excerpt_card(board, annotation)?;
        assert!(db.apply_vault_attachment(&stale, &backup).is_err());
        assert!(!db.preview_vault_attachment(note, id, &vault)?.actionable());
        assert_eq!(db.pdf_region(annotation)?, Some(region));
        assert_eq!(db.pdf_excerpt_source(card)?.unwrap().attachment_id, id);
        db.remove_pdf_annotation(annotation)?;
        assert!(db.pdf_excerpt_recovery_preview(card)?.is_some());
        assert!(!db.preview_vault_attachment(note, id, &vault)?.actionable());

        assert!(!backup.exists());
        assert_eq!(db.read_pdf_attachment(id)?.1, bytes.as_slice());
        assert_eq!(fs::read(&source)?, new_bytes);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
