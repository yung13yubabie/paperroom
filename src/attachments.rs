use super::{Notebook, Result};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

#[path = "attachment_bundle.rs"]
mod attachment_bundle;
#[cfg(windows)]
#[path = "win_attachment_publish.rs"]
mod win_attachment_publish;
use std::fs::{self, File};
use std::io::Read;
#[cfg(not(windows))]
use std::io::Write;
use std::path::Path;

pub(super) const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_STORAGE_BYTES: i64 = 128 * 1024 * 1024;
pub(super) const MAX_PER_NOTE: i64 = 16;
const MAX_ANNOTATIONS: i64 = 256;
const MAX_PDF_PAGE: u32 = 1_000_000;
const MAX_QUOTE: usize = 2048;
const MAX_COMMENT: usize = 4096;

pub struct Attachment {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub size: i64,
}

#[cfg(any(windows, test))]
pub(crate) struct CollectionAttachments {
    pub owner_token: String,
    pub files: Vec<Attachment>,
}

pub struct PdfAnnotation {
    pub id: i64,
    pub page: u32,
    pub quote: String,
    pub comment: String,
}

pub const MAX_PDF_SEARCH_QUERY: usize = 256;
const MAX_PDF_SEARCH_HITS: usize = 100;

pub struct PdfAnnotationHit {
    pub id: i64,
    pub note_id: i64,
    pub note_title: String,
    pub attachment_id: i64,
    pub attachment_name: String,
    pub page: u32,
    pub quote: String,
    pub comment: String,
}

const PDF_HITS: &str = "SELECT p.id,n.id,n.title,a.id,a.name,p.page,p.quote,p.comment
    FROM pdf_annotations p
    JOIN attachments a ON a.id=p.attachment_id
    JOIN notes n ON n.id=a.note_id
    WHERE a.kind='pdf' AND n.trash_root IS NULL";

fn pdf_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<PdfAnnotationHit> {
    Ok(PdfAnnotationHit {
        id: row.get(0)?,
        note_id: row.get(1)?,
        note_title: row.get(2)?,
        attachment_id: row.get(3)?,
        attachment_name: row.get(4)?,
        page: row.get(5)?,
        quote: row.get(6)?,
        comment: row.get(7)?,
    })
}

pub(super) const SCHEMA: &str = "
    CREATE TABLE attachments(
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        kind TEXT NOT NULL CHECK(kind IN ('pdf','png','jpeg','text','markdown')),
        content BLOB NOT NULL CHECK(length(content) BETWEEN 1 AND 8388608)
    );
    CREATE INDEX attachments_by_note ON attachments(note_id,id);
    CREATE TABLE pdf_annotations(
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        attachment_id INTEGER NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
        page INTEGER NOT NULL CHECK(page BETWEEN 1 AND 1000000),
        quote TEXT NOT NULL,
        comment TEXT NOT NULL
    );
    CREATE INDEX pdf_annotations_by_attachment ON pdf_annotations(attachment_id,page,id);
";

pub(super) fn file_kind(path: &Path) -> Result<&'static str> {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");
    match extension.to_ascii_lowercase().as_str() {
        "pdf" => Ok("pdf"),
        "png" => Ok("png"),
        "jpg" | "jpeg" => Ok("jpeg"),
        "txt" | "csv" | "base" => Ok("text"),
        "md" | "markdown" => Ok("markdown"),
        _ => Err("支援的附件格式為 PDF、PNG、JPEG、文字或 Markdown。".into()),
    }
}

pub(super) fn has_expected_signature(kind: &str, data: &[u8]) -> bool {
    match kind {
        "pdf" => {
            ((data.starts_with(b"%PDF-1.") && matches!(data.get(7), Some(b'0'..=b'7'))) || data.starts_with(b"%PDF-2.0"))
                && matches!(data.get(8), Some(b'\r' | b'\n'))
        }
        "png" => data.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpeg" => data.starts_with(b"\xff\xd8\xff"),
        "text" | "markdown" => !data.is_empty() && !data.contains(&0) && std::str::from_utf8(data).is_ok(),
        _ => false,
    }
}

#[cfg(windows)]
pub(super) fn require_local_volume(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

    let absolute = std::path::absolute(path)?;
    if let Some(Component::Prefix(prefix)) = absolute.components().next() {
        if let Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) = prefix.kind() {
            let root = format!("{}:\\", char::from(letter));
            let wide: Vec<_> = std::ffi::OsStr::new(&root).encode_wide().chain(Some(0)).collect();
            match unsafe { GetDriveTypeW(wide.as_ptr()) } {
                2 | 3 | 5 | 6 => return Ok(()), // Removable, fixed, optical or RAM disk.
                _ => {}
            }
        }
    }
    Err("附件請選用本機磁碟上的檔案。".into())
}

fn open_checked(path: &Path, directory: bool) -> Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        options.share_mode(FILE_SHARE_READ).custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT | if directory { FILE_FLAG_BACKUP_SEMANTICS } else { 0 },
        );
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0o400000 | 0o4000); // O_NOFOLLOW | O_NONBLOCK
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    };
    #[cfg(not(windows))]
    let reparse = metadata.file_type().is_symlink();
    if reparse || (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err("附件路徑須為一般檔案，且不可經過符號連結或重新分析點。".into());
    }
    Ok(file)
}

// Hold non-delete/non-write-shared handles for each directory before opening
// the next component to avoid a directory being replaced with a reparse point.
#[cfg(windows)]
fn pin_local_parents(path: &Path) -> Result<(std::path::PathBuf, Vec<File>)> {
    use std::path::Component;

    if path.components().any(|part| part == Component::ParentDir) {
        return Err("附件請選用完整的本機檔案路徑。".into());
    }
    let absolute = std::path::absolute(path)?;
    require_local_volume(&absolute)?;
    let mut ancestors: Vec<_> = absolute.ancestors().skip(1).collect();
    ancestors.reverse();
    let handles = ancestors.into_iter()
        .map(|parent| open_checked(parent, true))
        .collect::<Result<Vec<_>>>()?;
    Ok((absolute, handles))
}

pub(super) fn open_local_file(path: &Path) -> Result<(File, Vec<File>)> {
    #[cfg(windows)]
    let (absolute, parents) = pin_local_parents(path)?;
    #[cfg(windows)]
    let path = absolute.as_path();
    #[cfg(not(windows))]
    let parents = Vec::new();
    Ok((open_checked(path, false)?, parents))
}

pub(super) fn read_local_attachment_bytes(path: &Path) -> Result<Vec<u8>> {
    let (mut file, _parents) = open_local_file(path)?;
    let size = file.metadata()?.len();
    if size == 0 || size > MAX_FILE_BYTES as u64 {
        return Err("附件須為一般檔案，大小上限為 8 MiB。".into());
    }
    let mut content = Vec::new();
    Read::by_ref(&mut file).take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut content)?;
    if content.len() > MAX_FILE_BYTES {
        return Err("附件大小上限為 8 MiB。".into());
    }
    Ok(content)
}

pub(super) fn publish_attachment(path: &Path, content: &[u8]) -> Result<()> {
    #[cfg(windows)]
    {
        let (absolute, parents) = pin_local_parents(path)?;
        let parent = parents.last().ok_or("找不到目標資料夾。")?;
        let leaf = absolute.file_name().ok_or("找不到目標檔名。")?;
        win_attachment_publish::publish(parent, leaf, content)
    }
    #[cfg(not(windows))]
    crate::output::publish_new(path, |file, _| {
        file.write_all(content)?;
        Ok(())
    })
}

fn protect_base_source(db: &rusqlite::Connection, attachment: i64) -> Result<()> {
    let protected: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM base_card_receipts WHERE attachment=?1)", [attachment], |r| r.get(0),
    )?;
    if protected { return Err(super::note_history::BASE_SOURCE_GUIDANCE.into()); }
    Ok(())
}

impl Notebook {
    pub fn list_attachments(&self, note: i64) -> Result<Vec<Attachment>> {
        let mut query = self.connection.prepare(
            "SELECT a.id,a.name,a.kind,length(a.content)
             FROM attachments a JOIN notes n ON n.id=a.note_id
             WHERE a.note_id=?1 AND n.trash_root IS NULL ORDER BY a.id",
        )?;
        let rows = query.query_map([note], |row| {
            Ok(Attachment {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                size: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    #[cfg(any(windows, test))]
    pub(crate) fn collection_attachments(&self, collection: i64) -> Result<std::collections::HashMap<i64, CollectionAttachments>> {
        let mut query = self.connection.prepare(
            "SELECT a.note_id,i.token,a.id,a.name,a.kind,length(a.content)
             FROM collection_notes c JOIN notes n ON n.id=c.note_id
             JOIN note_link_identities i ON i.note_id=n.id JOIN attachments a ON a.note_id=n.id
             WHERE c.collection_id=?1 AND n.trash_root IS NULL ORDER BY a.id",
        )?;
        let mut files = std::collections::HashMap::<i64, CollectionAttachments>::new();
        let rows = query.query_map([collection], |row| Ok((row.get(0)?, row.get(1)?, Attachment {
            id: row.get(2)?,
            name: row.get(3)?,
            kind: row.get(4)?,
            size: row.get(5)?,
        })))?;
        for entry in rows {
            let (note, owner_token, file) = entry?;
            files.entry(note).or_insert_with(|| CollectionAttachments { owner_token, files: Vec::new() })
                .files.push(file);
        }
        Ok(files)
    }

    #[cfg(any(windows, test))]
    pub(crate) fn collection_attachment_bytes(&self, collection: i64, note: i64, owner: &str, file: &Attachment) -> Result<Vec<u8>> {
        self.connection.query_row(
            "SELECT a.content FROM collection_notes c JOIN notes n ON n.id=c.note_id
             JOIN note_link_identities i ON i.note_id=n.id JOIN attachments a ON a.note_id=n.id
             WHERE c.collection_id=?1 AND n.id=?2 AND i.token=?3 AND n.trash_root IS NULL
               AND a.id=?4 AND a.name=?5 COLLATE BINARY AND a.kind=?6 AND length(a.content)=?7",
            params![collection, note, owner, file.id, file.name, file.kind, file.size], |row| row.get(0),
        ).optional()?.ok_or_else(|| "資料列、筆記或附件已變更，請重新選取。".into())
    }

    #[cfg(any(windows, test))]
    pub(crate) fn save_collection_attachment(
        &self, collection: i64, note: i64, owner: &str, file: &Attachment, expected: &[u8], path: &Path,
    ) -> Result<()> {
        let actual = self.collection_attachment_bytes(collection, note, owner, file)?;
        if actual != expected {
            return Err("附件內容已變更，請重新選取。".into());
        }
        publish_attachment(path, &actual)
    }

    pub fn import_attachment(&mut self, note: i64, path: &Path) -> Result<i64> {
        let name = path.file_name().and_then(|value| value.to_str())
            .filter(|name| name.chars().count() <= 255 && !name.chars().any(char::is_control))
            .ok_or("附件需要有效的檔名。")?;
        let kind = file_kind(path)?;
        let content = read_local_attachment_bytes(path)?;
        if !has_expected_signature(kind, &content) {
            return Err("附件內容與選擇的檔案類型不符。".into());
        }

        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (active, count, used): (bool, i64, i64) = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM notes WHERE id=?1 AND trash_root IS NULL),
                    (SELECT count(*) FROM attachments WHERE note_id=?1),
                    (SELECT coalesce(sum(length(content)),0) FROM attachments)",
            [note],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if !active {
            return Err("找不到筆記，或筆記已移到垃圾桶。".into());
        }
        if count >= MAX_PER_NOTE || used > MAX_STORAGE_BYTES - content.len() as i64 {
            return Err("每篇筆記最多可放 16 個附件，工作區合計上限為 128 MiB。".into());
        }
        super::note_history_state::checkpoint(&tx, note)?;
        tx.execute(
            "INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,?2,?3,?4)",
            params![note, name, kind, content],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    #[cfg(any(windows, test))]
    pub fn read_pdf_attachment(&self, attachment: i64) -> Result<(String, Vec<u8>)> {
        self.connection.query_row(
            "SELECT a.name,a.content FROM attachments a JOIN notes n ON n.id=a.note_id
             WHERE a.id=?1 AND a.kind='pdf' AND n.trash_root IS NULL",
            [attachment],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?.ok_or("找不到可讀取的 PDF 附件。".into())
    }

    pub fn save_attachment_copy(&self, attachment: i64, path: &Path) -> Result<()> {
        let content: Vec<u8> = self.connection.query_row(
            "SELECT a.content FROM attachments a JOIN notes n ON n.id=a.note_id
             WHERE a.id=?1 AND n.trash_root IS NULL",
            [attachment],
            |row| row.get(0),
        ).optional()?.ok_or("找不到附件，或所屬筆記已移到垃圾桶。")?;
        publish_attachment(path, &content)
    }

    pub(crate) fn check_attachment_removal(&self, attachment: i64) -> Result<()> {
        protect_base_source(&self.connection, attachment)
    }

    pub fn remove_attachment(&self, attachment: i64) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let note: i64 = tx.query_row(
            "SELECT a.note_id FROM attachments a JOIN notes n ON n.id=a.note_id
             WHERE a.id=?1 AND n.trash_root IS NULL",
            [attachment], |row| row.get(0),
        ).optional()?.ok_or("找不到附件，或所屬筆記已移到垃圾桶。")?;
        protect_base_source(&tx, attachment)?;
        super::note_history_state::checkpoint(&tx, note)?;
        tx.execute("DELETE FROM attachments WHERE id=?1", [attachment])?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_pdf_annotations(&self, attachment: i64) -> Result<Vec<PdfAnnotation>> {
        let mut query = self.connection.prepare(
            "SELECT p.id,p.page,p.quote,p.comment
             FROM pdf_annotations p
             JOIN attachments a ON a.id=p.attachment_id
             JOIN notes n ON n.id=a.note_id
             WHERE p.attachment_id=?1 AND n.trash_root IS NULL ORDER BY p.page,p.id",
        )?;
        let rows = query.query_map([attachment], |row| {
            Ok(PdfAnnotation {
                id: row.get(0)?,
                page: row.get(1)?,
                quote: row.get(2)?,
                comment: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Returns up to 100 current-workspace hits and whether there are more.
    pub fn search_pdf_annotations(
        &self, query: &str, page: Option<u32>,
    ) -> Result<(Vec<PdfAnnotationHit>, bool)> {
        let query = query.trim();
        if query.contains('\0') || query.chars().count() > MAX_PDF_SEARCH_QUERY {
            return Err("搜尋文字最多 256 個字。".into());
        }
        if page.is_some_and(|value| !(1..=MAX_PDF_PAGE).contains(&value)) {
            return Err("PDF 頁碼須為 1 至 1,000,000。".into());
        }
        let sql = format!(
            "{PDF_HITS}
             AND (?2 IS NULL OR p.page=?2)
             AND (?1='' OR instr(lower(p.quote),lower(?1))>0
                        OR instr(lower(p.comment),lower(?1))>0
                        OR instr(lower(n.title),lower(?1))>0
                        OR instr(lower(a.name),lower(?1))>0)
             ORDER BY p.id DESC LIMIT ?3"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params![query, page, (MAX_PDF_SEARCH_HITS + 1) as i64], pdf_hit)?;
        let mut hits = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let truncated = hits.len() > MAX_PDF_SEARCH_HITS;
        hits.truncate(MAX_PDF_SEARCH_HITS);
        Ok((hits, truncated))
    }

    pub fn pdf_annotation_hit(&self, annotation: i64) -> Result<Option<PdfAnnotationHit>> {
        Ok(self.connection.query_row(
            &format!("{PDF_HITS} AND p.id=?1"),
            [annotation],
            pdf_hit,
        ).optional()?)
    }

    pub fn add_pdf_annotation(&self, attachment: i64, page: u32, quote: &str, comment: &str) -> Result<i64> {
        if !(1..=MAX_PDF_PAGE).contains(&page) {
            return Err("PDF 頁碼須為 1 至 1,000,000。".into());
        }
        let quote = quote.trim();
        let comment = comment.trim();
        if quote.contains('\0') || comment.contains('\0') || quote.chars().count() > MAX_QUOTE
            || comment.chars().count() > MAX_COMMENT || (quote.is_empty() && comment.is_empty())
        {
            return Err("請輸入引文或註解。引文上限 2,048 字，註解上限 4,096 字。".into());
        }
        let changed = self.connection.execute(
            "INSERT INTO pdf_annotations(attachment_id,page,quote,comment)
             SELECT a.id,?2,?3,?4 FROM attachments a JOIN notes n ON n.id=a.note_id
             WHERE a.id=?1 AND a.kind='pdf' AND n.trash_root IS NULL
             AND (SELECT count(*) FROM pdf_annotations WHERE attachment_id=a.id)<?5",
            params![attachment, page, quote, comment, MAX_ANNOTATIONS],
        )?;
        if changed == 0 {
            return Err("PDF 已移除、所屬筆記已移到垃圾桶，或註解已達 256 則上限。".into());
        }
        Ok(self.connection.last_insert_rowid())
    }

    pub fn remove_pdf_annotation(&self, annotation: i64) -> Result<()> {
        let changed = self.connection.execute(
            "DELETE FROM pdf_annotations WHERE id=?1 AND EXISTS(
                SELECT 1 FROM attachments a JOIN notes n ON n.id=a.note_id
                WHERE a.id=pdf_annotations.attachment_id AND n.trash_root IS NULL
             )",
            [annotation],
        )?;
        if changed == 0 {
            return Err("找不到註解，或所屬筆記已移到垃圾桶。".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_legacy_and_recent_anchors_in_the_correct_note_after_reopen() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-pdf-search-{}", std::process::id()));
        fs::create_dir(&dir)?;
        let source = dir.join("原稿.pdf");
        let path = dir.join("notes.sqlite3");
        let backup = dir.join("backup.sqlite3");
        fs::write(&source, b"%PDF-2.0\n")?;
        let (old_note, old_file, old, new_note, new_file, new, region) = {
            let mut db = Notebook::open(&path, false)?;
            let old_note = db.add(None, "共用標題", "")?;
            let new_note = db.add(None, "共用標題", "")?;
            let old_file = db.import_attachment(old_note, &source)?;
            let new_file = db.import_attachment(new_note, &source)?;
            // A plain text anchor from previous builds has no region row.
            let old = db.add_pdf_annotation(old_file, 1, "共用引文", "舊版 %_")?;
            let region = crate::store::PdfRegion { x: 0.1, y: 0.2, width: 0.5, height: 0.6 };
            let new = db.add_pdf_annotation_with_region(new_file, 2, "共用引文", "New marker", region)?;
            db.backup(&backup)?;
            (old_note, old_file, old, new_note, new_file, new, region)
        };
        for file in [&path, &backup] {
            let mut db = Notebook::open(file, false)?;
            let (both, truncated) = db.search_pdf_annotations("共用引文", None)?;
            assert!(!truncated);
            assert_eq!(both.iter().map(|hit| hit.id).collect::<Vec<_>>(), [new, old]);
            let page = db.search_pdf_annotations("", Some(2))?.0;
            assert_eq!((page[0].note_id, page[0].attachment_id, page[0].page), (new_note, new_file, 2));
            assert_eq!((page[0].note_title.as_str(),page[0].attachment_name.as_str()), ("共用標題", "原稿.pdf"));
            assert_eq!(db.search_pdf_annotations("NEW MARKER", Some(2))?.0[0].id, new);
            assert!(db.search_pdf_annotations("共用引文", Some(3))?.0.is_empty());
            assert!(db.search_pdf_annotations("", Some(0)).is_err());
            let legacy = db.search_pdf_annotations("%_", None)?.0;
            assert_eq!((legacy[0].id, legacy[0].note_id,legacy[0].attachment_id), (old, old_note, old_file));
            assert_eq!(db.pdf_region(legacy[0].id)?, None);
            assert_eq!(db.pdf_region(page[0].id)?, Some(region));
            db.trash(new_note)?;
            assert!(db.pdf_annotation_hit(new)?.is_none());
            assert!(db.search_pdf_annotations("", Some(2))?.0.is_empty());
            db.restore(new_note)?;
            assert_eq!(db.pdf_annotation_hit(new)?.unwrap().id, new);
            db.remove_attachment(new_file)?;
            assert!(db.pdf_annotation_hit(new)?.is_none());
            assert_eq!(db.pdf_annotation_hit(old)?.unwrap().note_id, old_note);
        }
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn validates_pdf_header_without_claiming_to_parse_it() {
        assert!(has_expected_signature("pdf", b"%PDF-1.7\n"));
        assert!(has_expected_signature("pdf", b"%PDF-2.0\r\n"));
        for invalid in [b"%PDF-1.9\n".as_slice(), b"junk%PDF-1.7\n", b"%PDF-1.7", b"%PDF-2.0x\n"] {
            assert!(!has_expected_signature("pdf", invalid));
        }
        assert!(!has_expected_signature("text", b"before\0after"));
        assert!(!has_expected_signature("png", b"not PNG"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn never_follows_the_selected_source_symlink() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-attachment-link-{}", std::process::id()));
        fs::create_dir(&dir)?;
        let target = dir.join("original.pdf");
        let link = dir.join("linked.pdf");
        fs::write(&target, b"%PDF-2.0\n")?;
        std::os::unix::fs::symlink(&target, &link)?;
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let note = db.add(None, "閱讀", "")?;
        assert!(db.import_attachment(note, &link).is_err());
        assert!(db.list_attachments(note)?.is_empty());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn backup_preserves_bytes_and_page_anchors_and_stale_ids_do_not_rebind() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-attachment-test-{}", std::process::id()));
        fs::create_dir(&dir)?;
        let database = dir.join("notes.sqlite3");
        let input = dir.join("文件.pdf");
        let output = dir.join("副本.pdf");
        let backup = dir.join("backup.sqlite3");
        let sample = b"%PDF-1.7\n\x00\x80\xff\n";
        fs::write(&input, sample)?;
        {
            let mut first = Notebook::open(&database, false)?;
            let mut second = Notebook::open(&database, false)?;
            let note = first.add(None, "閱讀", "")?;
            let file = first.import_attachment(note, &input)?;
            let annotation = first.add_pdf_annotation(file, 7, "引文", "自己的筆記")?;
            let files = first.list_attachments(note)?;
            assert_eq!(files.len(), 1);
            assert_eq!((files[0].name.as_str(), files[0].kind.as_str(), files[0].size),
                       ("文件.pdf", "pdf", sample.len() as i64));
            assert!(first.add_pdf_annotation(file, 0, "引文", "").is_err());
            first.trash(note)?;
            assert!(second.import_attachment(note, &input).is_err());
            assert!(second.add_pdf_annotation(file, 4, "引文", "").is_err());
            assert!(second.remove_pdf_annotation(annotation).is_err());
            assert!(second.remove_attachment(file).is_err());
            assert!(second.save_attachment_copy(file, &output).is_err());
            assert!(second.list_attachments(note)?.is_empty());
            assert!(second.list_pdf_annotations(file)?.is_empty());
            first.restore(note)?;
            first.backup(&backup)?;
            let restored = Notebook::open(&backup, false)?;
            restored.save_attachment_copy(file, &output)?;
            assert_eq!(fs::read(&output)?, sample);
            assert!(restored.save_attachment_copy(file, &output).is_err());
            let anchors = restored.list_pdf_annotations(file)?;
            assert_eq!(anchors.len(), 1);
            assert_eq!((anchors[0].id, anchors[0].page, anchors[0].quote.as_str(), anchors[0].comment.as_str()),
                       (annotation, 7, "引文", "自己的筆記"));
            first.remove_attachment(file)?;
            assert!(second.list_pdf_annotations(file)?.is_empty());
            let newer = first.import_attachment(note, &input)?;
            let new_anchor = first.add_pdf_annotation(newer, 9, "", "新註解")?;
            assert_ne!(newer, file);
            assert_ne!(new_anchor, annotation);
            assert!(second.remove_attachment(file).is_err());
            assert!(second.remove_pdf_annotation(annotation).is_err());
            assert_eq!(second.list_pdf_annotations(newer)?[0].page, 9);
        }
        fs::remove_dir_all(dir)?;
        Ok(())
    }
    #[test]
    fn collection_export_checks_exact_bytes_and_membership_before_creating_a_copy() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("workspace.sqlite3");
        let mut notebook = Notebook::open(&path, false)?;
        let writer = Notebook::open(&path, false)?;
        let collection = notebook.create_collection("Sources")?;
        let a = notebook.add(None, "同名", "Current")?;
        let b = notebook.add(None, "同名", "Other")?;
        notebook.add_to_collection(collection, a, "保留")?;
        notebook.add_to_collection(collection, b, "保留")?;
        let source = dir.join("same.txt");
        fs::write(&source, b"first file")?;
        notebook.import_attachment(a, &source)?;
        fs::write(&source, b"other file")?;
        notebook.import_attachment(b, &source)?;
        let receipts = notebook.collection_attachments(collection)?;
        let receipt = &receipts[&a];
        let file = &receipt.files[0];
        let data = notebook.collection_attachment_bytes(collection, a, &receipt.owner_token, file)?;
        assert_eq!(data, b"first file");
        let destination = dir.join("copy.txt");
        assert!(notebook.save_collection_attachment(collection, b, &receipts[&b].owner_token, file, &data, &destination).is_err());
        assert!(!destination.exists());
        writer.connection.execute("UPDATE attachments SET content=?2 WHERE id=?1", params![file.id,b"newer file"])?;
        assert!(notebook.save_collection_attachment(collection, a, &receipt.owner_token, file, &data, &destination).is_err());
        assert!(!destination.exists());
        let data = notebook.collection_attachment_bytes(collection, a, &receipt.owner_token, file)?;
        notebook.save_collection_attachment(collection, a, &receipt.owner_token, file, &data, &destination)?;
        assert_eq!(fs::read(destination)?, b"newer file");
        writer.remove_from_collection(collection, a, "保留")?;
        let missing = dir.join("removed.txt");
        assert!(notebook.save_collection_attachment(collection, a, &receipt.owner_token, file, &data, &missing).is_err());
        assert!(!missing.exists());
        drop(writer);
        drop(notebook);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

}
