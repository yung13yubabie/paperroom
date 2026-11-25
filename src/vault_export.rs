use super::{MAX_NOTE_BYTES, Notebook, Result};
use super::attachments::{MAX_FILE_BYTES, file_kind, has_expected_signature};
use super::migration_media::{self, MediaPaths};
use crate::archive::MAX_ENTRIES;
use crate::output::{publish_new, windows_output_name};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet, HashMap};
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

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct VaultExport {
    pub pages: usize,
    pub files: usize,
    pub unbound: usize,
}

#[derive(Default)]
struct SourceAssets {
    files: BTreeMap<String, i64>,
    bindings: HashMap<i64, BTreeSet<String>>,
    unbound: usize,
}

impl SourceAssets {
    fn collect(db: &Connection, root: i64, paths: &mut PortablePaths) -> Result<Self> {
        let mut query = db.prepare(
            "SELECT a.note_id,a.id,o.relative_path,a.kind,a.content
             FROM vault_paths p JOIN notes n ON n.id=p.note_id
             JOIN notes root ON root.id=p.import_root JOIN attachments a ON a.note_id=n.id
             LEFT JOIN attachment_origins o ON o.attachment_id=a.id
             WHERE p.import_root=?1 AND root.trash_root IS NULL AND n.trash_root IS NULL
             ORDER BY a.id",
        )?;
        let mut same_bytes = db.prepare("SELECT content=?2 AND kind=?3 FROM attachments WHERE id=?1")?;
        let mut rows = query.query([root])?;
        let mut out = Self::default();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let Some(path) = row.get_ref(2)?.as_str_or_null()? else {
                out.unbound += 1;
                continue;
            };
            let note = row.get(0)?;
            let id = row.get(1)?;
            let kind = row.get_ref(3)?.as_str()?;
            let data = row.get_ref(4)?.as_blob()?;
            if !migration_media::supported(Path::new(path)) || file_kind(Path::new(path))? != kind
                || data.is_empty() || data.len() > MAX_FILE_BYTES || !has_expected_signature(kind, data)
            {
                return Err("來源附件的路徑、格式或容量與筆記庫匯出規格不符。".into());
            }
            if let Some(existing) = out.files.get(path) {
                let same: bool = same_bytes.query_row(params![existing, data, kind], |row| row.get(0))?;
                if !same {
                    return Err(format!("來源附件「{path}」有內容不同的副本。請先釐清來源，再重新匯出。").into());
                }
            } else {
                paths.add_file(path)?;
                out.files.insert(path.to_owned(), id);
            }
            bytes += data.len();
            if bytes > super::attachments::MAX_STORAGE_BYTES as usize {
                return Err("來源附件合計超過可回匯的 128 MiB 上限。".into());
            }
            if !out.bindings.entry(note).or_default().insert(path.to_owned()) {
                return Err(format!("單篇筆記有多筆相同來源路徑的附件「{path}」。請先核對附件，再重新匯出。").into());
            }
        }
        Ok(out)
    }

    fn verify_links(&self, resolver: &MediaPaths<'_>, note: i64, path: &str, body: &str) -> Result<()> {
        let parts = path.split('/').map(str::to_owned).collect::<Vec<_>>();
        let expected = self.bindings.get(&note);
        let mut matched = BTreeSet::new();
        for (target, wiki) in migration_media::referenced_paths(body) {
            if let Some(resolved) = resolver.resolve(&parts, &target, wiki) {
                if wiki && !target.contains('/') {
                    return Err(format!("「{path}」含以檔名搜尋的附件參照「{target}」。請指定附件的資料夾路徑，再重新匯出。").into());
                }
                if !expected.is_some_and(|paths| paths.contains(&resolved)) {
                    return Err(format!("回匯會改變「{path}」的附件配對。請先核對該筆記的來源附件。").into());
                }
                matched.insert(resolved);
            }
        }
        if expected.is_some_and(|paths| paths != &matched) {
            return Err(format!("「{path}」的來源附件缺少對應的正文連結。請先補正連結，再重新匯出。").into());
        }
        Ok(())
    }
}

impl Notebook {
    pub fn export_vault_markdown(&self, import_root: i64, destination: &Path) -> Result<usize> {
        Ok(self.export_vault(import_root, destination, false)?.pages)
    }

    pub(crate) fn export_vault_with_media(&self, root: i64, destination: &Path) -> Result<VaultExport> {
        self.export_vault(root, destination, true)
    }

    fn export_vault(&self, import_root: i64, destination: &Path, include_media: bool) -> Result<VaultExport> {
        // Paths and text come from one SQLite snapshot. Only the explicitly
        // selected import contributes files to the new archive.
        let tx = self.connection.unchecked_transaction()?;
        let mut report = VaultExport::default();
        publish_new(destination, |file, _| {
            {
                let mut statement = tx.prepare(
                    "SELECT p.relative_path,n.body,n.id FROM vault_paths p
                     JOIN notes n ON n.id=p.note_id
                     JOIN notes root ON root.id=p.import_root
                     WHERE p.import_root=?1 AND root.trash_root IS NULL AND n.trash_root IS NULL
                     ORDER BY p.relative_path COLLATE BINARY",
                )?;
                let mut rows = statement.query([import_root])?;
                let mut zip = zip::ZipWriter::new(file);
                let mut paths = PortablePaths::default();
                let assets = if include_media { Some(SourceAssets::collect(&tx, import_root, &mut paths)?) }
                    else { None };
                let resolver = assets.as_ref().map(|a| MediaPaths::from_paths(a.files.keys().map(String::as_str)));
                let mut bytes = 0;

                while let Some(row) = rows.next()? {
                    let path = row.get_ref(0)?.as_str()?;
                    let body = row.get_ref(1)?.as_str()?;
                    paths.add_file(path)?;
                    if !Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md")) {
                        return Err("筆記庫路徑的檔案必須是 Markdown。".into());
                    }
                    if body.len() > MAX_NOTE_BYTES || body.contains('\0') || body.len() > MAX_TOTAL_BYTES - bytes {
                        return Err("Markdown 匯出上限為單篇 4 MiB、合計 80 MiB。".into());
                    }
                    if let Some((assets, resolver)) = assets.as_ref().zip(resolver.as_ref()) {
                        assets.verify_links(resolver, row.get(2)?, path, body)?;
                    }
                    bytes += body.len();
                    zip.start_file(path, SimpleFileOptions::default())?;
                    zip.write_all(body.as_bytes())?;
                    report.pages += 1;
                }
                if report.pages == 0 {
                    return Err("這個匯入的筆記庫目前沒有可匯出的 Markdown 筆記。".into());
                }
                if let Some(assets) = assets {
                    let mut data = tx.prepare("SELECT content FROM attachments WHERE id=?1")?;
                    for (path, id) in assets.files {
                        let bytes: Vec<u8> = data.query_row([id], |r| r.get(0))?;
                        zip.start_file(path, SimpleFileOptions::default())?;
                        zip.write_all(&bytes)?;
                        report.files += 1;
                    }
                    report.unbound = assets.unbound;
                }
                zip.finish()?;
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok(report)
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

    #[test]
    fn vault_media_export_roundtrips_bytes_and_refuses_silent_rebinding() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let source = dir.join("來源");
        let extracted = dir.join("解壓");
        fs::create_dir_all(source.join("段落"))?;
        fs::create_dir_all(source.join("附件"))?;
        let first = "\u{feff}# 鏡頭\r\n\r\n![封面](../附件/封面.png)\r\n![[附件/封面.png]]\r\n[引用](../附件/引用.txt)\r\n";
        let second = "[引用](附件/引用.txt)\n";
        let sources = [("段落/鏡頭.md", first.as_bytes()), ("索引.md", second.as_bytes()),
                       ("附件/引用.txt", "原文\r\n".as_bytes())];
        for (path, bytes) in sources { fs::write(source.join(path), bytes)?; }
        image::RgbImage::from_pixel(1, 1, image::Rgb([170, 80, 40])).save(source.join("附件/封面.png"))?;

        let mut db = Notebook::open(&dir.join("notes.db"), false)?;
        let mut import = |folder: &Path, title: &str| -> Result<i64> {
            let vault = crate::vault::read(folder)?;
            let media = migration_media::plan(folder, &vault)?;
            db.import_vault_with_media(title, vault.pages, media)
        };
        let root = import(&source, "同名匯入")?;
        let duplicate = import(&source, "同名匯入")?;
        assert_ne!(root, duplicate);
        let note = db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path='索引.md'",
            [root], |r| r.get::<_, i64>(0))?;
        db.connection.execute("INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,'手動.txt','text',?2)",
            params![note, b"manual".as_slice()])?;
        let output = dir.join("可攜.zip");
        assert_eq!(db.export_vault_with_media(root, &output)?,
            VaultExport { pages: 2, files: 2, unbound: 1 });
        let mut zip = zip::ZipArchive::new(File::open(&output)?)?;
        assert_eq!(zip.len(), 4);
        zip.extract(&extracted)?;
        for (path, original) in sources {
            assert_eq!(fs::read(extracted.join(path))?, original);
        }
        assert_eq!(fs::read(extracted.join("附件/封面.png"))?, fs::read(source.join("附件/封面.png"))?);
        let vault = crate::vault::read(&extracted)?;
        let media = migration_media::plan(&extracted, &vault)?;
        assert_eq!((media.count(), media.file_count(), media.unresolved), (3, 2, 0));
        let reimport = db.import_vault_with_media("回匯", vault.pages, media)?;
        let other: String = db.connection.query_row(
            "SELECT n.body FROM vault_paths p JOIN notes n ON n.id=p.note_id
             WHERE p.import_root=?1 AND p.relative_path='索引.md'",
            [reimport], |r| r.get(0),
        )?;
        assert_eq!(other, second);

        let before = fs::read(&output)?;
        assert!(db.export_vault_with_media(duplicate, &output).is_err());
        assert_eq!(fs::read(&output)?, before);
        let bad = dir.join("拒絕.zip");
        db.connection.execute("UPDATE attachments SET content=?2 WHERE note_id=?1 AND name='引用.txt'",
            params![note, b"different".as_slice()])?;
        let error = db.export_vault_with_media(root, &bad).unwrap_err();
        assert!(error.to_string().contains("內容不同的副本"));
        assert!(!bad.exists());
        db.connection.execute("UPDATE attachments SET content=?2 WHERE note_id=?1 AND name='引用.txt'",
            params![note, "原文\r\n".as_bytes()])?;
        let saved = db.note(note)?.unwrap();
        db.save(note, saved.version, &saved.row.title, "[無資料夾](missing.txt)\n")?;
        let error = db.export_vault_with_media(root, &bad).unwrap_err();
        assert!(error.to_string().contains("缺少對應"));
        assert!(!bad.exists());
        let saved = db.note(note)?.unwrap();
        db.save(note, saved.version, &saved.row.title, "![[引用.txt]]")?;
        assert!(db.export_vault_with_media(root, &bad).unwrap_err().to_string().contains("以檔名搜尋"));
        assert!(!bad.exists());
        assert!(!fs::read_dir(&dir)?.any(|entry| entry.is_ok_and(|e|
            e.file_name().to_string_lossy().ends_with(".partial"))));
        drop(zip);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
