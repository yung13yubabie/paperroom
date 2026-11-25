//! Revalidate the exact data disclosed by the folder import preview before saving.
use super::{Result, migration_media::{self, MediaPlan}, snapshot_sha256::sha256};
use crate::vault::{self, Vault};
use std::path::{Path, PathBuf};

pub(crate) struct VaultFolderReview {
    root: PathBuf,
    fingerprint: [u8; 32],
    report: String,
}

impl VaultFolderReview {
    pub(crate) fn read(root: &Path) -> Result<Self> {
        let (vault, media) = read_source(root)?;
        let report = report(&vault, &media);
        Ok(Self { root: root.to_owned(), fingerprint: fingerprint(&vault, &media, &report), report })
    }

    pub(crate) fn report(&self) -> &str {
        &self.report
    }

    pub(crate) fn verify(&self) -> Result<(Vault, MediaPlan)> {
        let (vault, media) = read_source(&self.root)
            .map_err(|error| format!("無法再次核對 Vault 資料夾，請重新預覽。\r\n{error}"))?;
        if fingerprint(&vault, &media, &report(&vault, &media)) != self.fingerprint {
            return Err("預覽後 Vault 來源已有變更。請重新開啟預覽確認。".into());
        }
        Ok((vault, media))
    }
}

fn read_source(root: &Path) -> Result<(Vault, MediaPlan)> {
    let vault = vault::read(root)?;
    let media = migration_media::plan(root, &vault)?;
    Ok((vault, media))
}

fn report(vault: &Vault, media: &MediaPlan) -> String {
    media.report(vault.pages.len(), vault.skipped.saturating_sub(media.file_count()))
}

fn fingerprint(vault: &Vault, media: &MediaPlan, report: &str) -> [u8; 32] {
    // Hash the length-delimited list of files/bindings rather than retaining
    // the full preview in memory while the confirmation dialog is open.
    let mut summary = Vec::new();
    summary.extend_from_slice(&(vault.pages.len() as u64).to_le_bytes());
    for page in &vault.pages {
        summary.extend_from_slice(&sha256(page.parts.join("/").as_bytes()));
        summary.extend_from_slice(&sha256(page.title.as_bytes()));
        summary.extend_from_slice(&sha256(page.body.as_bytes()));
    }
    media.append_review_fingerprints(&mut summary);
    summary.extend_from_slice(&sha256(report.as_bytes()));
    sha256(&summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Notebook;
    use std::fs;

    #[test]
    fn folder_review_checks_real_bytes_and_reports_ambiguous_paths_before_import() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let folder = dir.join("Vault");
        let notes = folder.join("場");
        let assets = folder.join("素材");
        fs::create_dir_all(&notes)?;
        fs::create_dir_all(&assets)?;
        let body = "\u{feff}[相對頁](./B.md#review)\r\n[雙義頁](B.md#review)\r\n[附件](../素材/卡.txt)\r\n[缺檔](../素材/缺.txt)";
        let note_path = notes.join("A.md");
        let asset_path = assets.join("卡.txt");
        fs::write(&note_path, body)?;
        fs::write(notes.join("B.md"), "# Review\n場內的同名筆記")?;
        fs::write(folder.join("B.md"), "# Review\n根目錄的同名筆記")?;
        fs::write(&asset_path, b"asset one")?;
        let db_path = dir.join("workspace.sqlite3");
        let mut db = Notebook::open(&db_path, false)?;
        let before = fs::read(&db_path)?;
        let review = VaultFolderReview::read(&folder)?;
        let report = review.report();
        assert!(report.contains("附件綁定 1 筆，未解析 1 筆"));
        assert!(report.contains("相符 1 筆，未解析 1 筆"));
        assert!(report.contains("相對於筆記與相對於 Vault 根目錄時指向不同筆記"));
        assert!(report.contains("找不到可匯入的來源檔案"));
        assert_eq!(fs::read(&db_path)?, before);

        fs::write(&asset_path, b"asset two")?;
        assert!(review.verify().err().unwrap().to_string().contains("來源已有變更"));
        fs::write(&asset_path, b"asset one")?;
        fs::write(&note_path, format!("{body}新增正文"))?;
        assert!(review.verify().err().unwrap().to_string().contains("來源已有變更"));
        assert_eq!(fs::read(&db_path)?, before);
        fs::write(&note_path, body)?;
        let (source, media) = review.verify()?;
        let root = db.import_vault_with_media("合成", source.pages, media)?;
        drop(db);

        let db = Notebook::open(&db_path, false)?;
        let get = |path| db.connection.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            rusqlite::params![root, path], |row| row.get::<_, i64>(0));
        let a = db.note(get("場/A.md")?)?.unwrap();
        assert_eq!(a.body, body);
        let links = db.vault_note_links(&a)?;
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].note_id, get("場/B.md")?);
        let attachment: (String, Vec<u8>) = db.connection.query_row(
            "SELECT o.relative_path,a.content FROM attachments a JOIN attachment_origins o ON o.attachment_id=a.id
              WHERE a.note_id=?1", [a.row.id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        assert_eq!(attachment, ("素材/卡.txt".to_owned(), b"asset one".to_vec()));
        assert_eq!(fs::read_to_string(note_path)?, body);
        assert_eq!(fs::read(asset_path)?, b"asset one");
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn empty_or_removed_vault_fails_read_only_review() -> Result<()> {
        let dir = super::super::tests::data_dir();
        assert!(VaultFolderReview::read(&dir).err().unwrap().to_string().contains("沒有可匯入"));
        let file = dir.join("A.md");
        fs::write(&file, "原文")?;
        let review = VaultFolderReview::read(&dir)?;
        fs::remove_file(file)?;
        assert!(review.verify().err().unwrap().to_string().contains("請重新預覽"));
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
