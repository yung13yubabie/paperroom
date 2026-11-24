use super::{MAX_NOTE_BYTES, Notebook, Result};
use crate::archive::MAX_ENTRIES;
use crate::output::{publish_new, windows_output_name};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;

const MAX_TOTAL_BYTES: usize = 80 * 1024 * 1024;
const MAX_PATH_DEPTH: usize = 64;

struct Entry {
    name: String,
    id: usize,
    directory: bool,
}

#[derive(Default)]
struct PortablePaths(HashMap<(usize, String), Entry>);

impl PortablePaths {
    fn add_file(&mut self, path: &str) -> Result<()> {
        let mut components = path.split('/').peekable();
        let mut parent = 0;
        let mut depth = 0;
        while let Some(component) = components.next() {
            depth += 1;
            let directory = components.peek().is_some();
            if depth > MAX_PATH_DEPTH || component.starts_with('.') || component.encode_utf16().count() > 255 {
                return Err("筆記庫包含無法安全匯出的路徑。".into());
            }
            windows_output_name(OsStr::new(component))?;
            if !directory && !component.get(component.len().saturating_sub(3)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".md"))
            {
                return Err("筆記庫路徑的檔案必須是 Markdown。".into());
            }
            let next_id = self.0.len() + 1;
            let key = (parent, component.to_uppercase());
            match self.0.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    let existing = entry.get();
                    if !directory || !existing.directory || existing.name != component {
                        return Err("筆記庫路徑有名稱、大小寫或檔案／資料夾衝突。".into());
                    }
                    parent = existing.id;
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    if next_id > MAX_ENTRIES {
                        return Err("筆記庫匯出上限為 15,000 個檔案及資料夾。".into());
                    }
                    entry.insert(Entry { name: component.to_owned(), id: next_id, directory });
                    parent = next_id;
                }
            }
        }
        Ok(())
    }
}

impl Notebook {
    pub fn export_vault_markdown(&self, import_root: i64, destination: &Path) -> Result<usize> {
        // Paths and text come from one SQLite snapshot. Only the explicitly
        // selected import contributes files to the new archive.
        let tx = self.connection.unchecked_transaction()?;
        let mut count = 0;
        publish_new(destination, |file, _| {
            {
                let mut statement = tx.prepare(
                    "SELECT p.relative_path,n.body FROM vault_paths p
                     JOIN notes n ON n.id=p.note_id
                     JOIN notes root ON root.id=p.import_root
                     WHERE p.import_root=?1 AND root.trash_root IS NULL AND n.trash_root IS NULL
                     ORDER BY p.relative_path COLLATE BINARY",
                )?;
                let mut rows = statement.query([import_root])?;
                let mut zip = zip::ZipWriter::new(file);
                let mut paths = PortablePaths::default();
                let mut bytes = 0;

                while let Some(row) = rows.next()? {
                    let path = row.get_ref(0)?.as_str()?;
                    let body = row.get_ref(1)?.as_str()?;
                    paths.add_file(path)?;
                    if body.len() > MAX_NOTE_BYTES || body.contains('\0') || body.len() > MAX_TOTAL_BYTES - bytes {
                        return Err("Markdown 匯出上限為單篇 4 MiB、合計 80 MiB。".into());
                    }
                    bytes += body.len();
                    zip.start_file(path, SimpleFileOptions::default())?;
                    zip.write_all(body.as_bytes())?;
                    count += 1;
                }
                if count == 0 {
                    return Err("這個匯入的筆記庫目前沒有可匯出的 Markdown 筆記。".into());
                }
                zip.finish()?;
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use std::fs::{self, File};
    use std::io::Read;

    #[test]
    fn selected_vault_zip_preserves_original_paths_and_latest_bytes() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let output = dir.join("筆記庫.zip");
        let mut db = Notebook::open(&dir.join("notes.sqlite3"), false)?;
        let page = |folder: &str, body: &str| ImportedPage {
            parts: vec![folder.into(), "鏡頭.MD".into()], title: "鏡頭".into(), body: body.into(),
        };
        let first_body = "\u{feff}---\r\n標籤: 測試\r\n---\r\n[[../乙/鏡頭#取景]]\r\n";
        let first = db.import_vault("同名", vec![page("甲", first_body), page("乙", "## 取景\n前情"), page("另頁", "略過")])?;
        let other = db.import_vault("同名", vec![page("甲", "另一個筆記庫")])?;
        let note = |path: &str| db.connection.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            (first, path), |row| row.get::<_, i64>(0),
        );
        let edited = note("乙/鏡頭.MD")?;
        db.save(edited, db.note(edited)?.unwrap().version, "紙間顯示新標題", "## 取景\n更新")?;
        db.trash(note("另頁/鏡頭.MD")?)?;
        assert_eq!(db.export_vault_markdown(first, &output)?, 2);

        let mut zip = zip::ZipArchive::new(File::open(&output)?)?;
        let read = |zip: &mut zip::ZipArchive<File>, name: &str| -> Result<Vec<u8>> {
            let mut body = Vec::new();
            zip.by_name(name)?.read_to_end(&mut body)?;
            Ok(body)
        };
        assert_eq!(read(&mut zip, "甲/鏡頭.MD")?, first_body.as_bytes());
        assert_eq!(read(&mut zip, "乙/鏡頭.MD")?, "## 取景\n更新".as_bytes());
        drop(zip);
        let prior = fs::read(&output)?;
        assert!(db.export_vault_markdown(other, &output).is_err());
        assert_eq!(fs::read(&output)?, prior);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn unsafe_or_colliding_legacy_paths_leave_no_partial_export() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let output = dir.join("拒絕.zip");
        let mut db = Notebook::open(&dir.join("notes.sqlite3"), false)?;
        let root = db.import_vault("測試", ["Original.md", "unsafe.md"].into_iter().map(|name| ImportedPage {
            parts: vec![name.into()], title: name.into(), body: "內容".into(),
        }).collect())?;
        let replace = |path: &str| db.connection.execute(
            "UPDATE vault_paths SET relative_path=?1 WHERE import_root=?2 AND relative_path<>'Original.md'",
            (path, root),
        );
        for path in ["original.md", "../escape.md", "AUX/筆記.md", ".obsidian/設定.md",
                     "Original.md/巢狀.md", "z/../escape.md", r"路徑\backslash.md", "drive:stream.md"] {
            replace(path)?;
            assert!(db.export_vault_markdown(root, &output).is_err(), "{path}");
            assert!(!output.exists());
            assert!(!fs::read_dir(&dir)?.any(|entry| {
                entry.is_ok_and(|entry| entry.file_name().to_string_lossy().ends_with(".partial"))
            }));
        }

        let mut paths = PortablePaths::default();
        paths.add_file("Notes/First.md")?;
        assert!(paths.add_file("notes/Second.md").is_err());
        paths.add_file("Notes/Second.md")?;
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
