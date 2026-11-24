use super::{MAX_FILE_BYTES, MAX_PER_NOTE, MAX_STORAGE_BYTES, file_kind, has_expected_signature};
use crate::output::{publish_new, windows_output_name};
use crate::store::{MAX_NOTE_BYTES, Notebook, Result, clean_title, insert, validate_body};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const FORMAT: &str = "paperroom-note-1";
const MANIFEST: &str = "paperroom-note.json";
const MARKDOWN: &str = "筆記.md";
const INDEX: &str = "附件索引.md";
const MAX_MANIFEST: usize = 64 * 1024;
const MAX_INDEX: usize = 64 * 1024;
const MAX_ARCHIVE: u64 = (MAX_STORAGE_BYTES as u64) + (MAX_NOTE_BYTES as u64) + 4 * 1024 * 1024;

struct Entry {
    name: String,
    kind: &'static str,
    size: usize,
}

impl Entry {
    fn new(name: &str, size: usize) -> Result<Self> {
        windows_output_name(OsStr::new(name))?;
        if name.encode_utf16().count() > 255 || name.chars().any(char::is_control)
            || size == 0 || size > MAX_FILE_BYTES
        {
            return Err("附件名稱或大小超出紙間 ZIP 的支援範圍。".into());
        }
        Ok(Self { name: name.into(), kind: file_kind(Path::new(name))?, size })
    }

    fn path(&self, index: usize) -> String {
        format!("attachments/{:02}/{}", index + 1, self.name)
    }
}

fn note_title(title: &str) -> Result<&str> {
    if title.contains('\0') || clean_title(title) != title {
        return Err("紙間 ZIP 需要有效的筆記標題。".into());
    }
    Ok(title)
}

fn uri_path(path: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn attachment_index(entries: &[Entry]) -> String {
    let mut markdown = "# 附件索引\n\n".to_owned();
    use std::fmt::Write;
    for (index, entry) in entries.iter().enumerate() {
        let _ = writeln!(markdown, "- [附件 {}]({})", index + 1, uri_path(&entry.path(index)));
    }
    markdown
}

fn write_entry(writer: &mut ZipWriter<impl Write + Seek>, path: &str, data: &[u8]) -> Result<()> {
    writer.start_file(path, SimpleFileOptions::default())?;
    writer.write_all(data)?;
    Ok(())
}

fn read_entry(archive: &mut ZipArchive<File>, path: &str, limit: usize) -> Result<Vec<u8>> {
    let mut file = archive.by_name(path)?;
    if !file.is_file() || file.is_symlink() || file.encrypted() || file.size() > limit as u64 {
        return Err("紙間 ZIP 內有不支援的項目或超出大小限制。".into());
    }
    let size = file.size();
    let mut bytes = Vec::new();
    file.by_ref().take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != size || bytes.len() > limit {
        return Err("紙間 ZIP 項目大小與宣告不符。".into());
    }
    Ok(bytes)
}

fn parse_manifest(bytes: &[u8]) -> Result<(String, Vec<Entry>)> {
    let value: Value = serde_json::from_slice(bytes)?;
    if value["format"].as_str() != Some(FORMAT) {
        return Err("此 ZIP 不是支援的紙間筆記與附件格式。".into());
    }
    let title = note_title(value["title"].as_str().ok_or("紙間 ZIP 缺少筆記標題。")?)?;
    let files = value["attachments"].as_array().ok_or("紙間 ZIP 缺少附件清單。")?;
    if files.len() > MAX_PER_NOTE as usize {
        return Err("每篇筆記最多 16 個附件。".into());
    }
    let mut entries = Vec::with_capacity(files.len());
    let mut total = 0;
    for file in files {
        let name = file["name"].as_str().ok_or("紙間 ZIP 附件缺少名稱。")?;
        let size = file["bytes"].as_u64().ok_or("紙間 ZIP 附件缺少大小。")?;
        let entry = Entry::new(name, usize::try_from(size)?)?;
        total += entry.size;
        if total > MAX_STORAGE_BYTES as usize {
            return Err("紙間 ZIP 附件總量超出 128 MiB。".into());
        }
        entries.push(entry);
    }
    Ok((title.to_owned(), entries))
}

impl Notebook {
    pub fn export_note_bundle(&self, note: i64, destination: &Path) -> Result<usize> {
        let tx = self.connection.unchecked_transaction()?;
        let (title, body): (String, String) = tx.query_row(
            "SELECT title,body FROM notes WHERE id=?1 AND trash_root IS NULL",
            [note], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?.ok_or("找不到可匯出的筆記。")?;
        note_title(&title)?;
        validate_body(&body)?;
        let mut count = 0;
        publish_new(destination, |output, _| {
            {
                let mut writer = ZipWriter::new(output);
                let mut statement = tx.prepare(
                    "SELECT name,kind,content FROM attachments WHERE note_id=?1 ORDER BY id",
                )?;
                let mut query = statement.query([note])?;
                let mut entries = Vec::new();
                let mut attached_bytes = 0;
                while let Some(row) = query.next()? {
                    let name = row.get_ref(0)?.as_str()?;
                    let kind = row.get_ref(1)?.as_str()?;
                    let content = row.get_ref(2)?.as_blob()?;
                    let entry = Entry::new(name, content.len())?;
                    attached_bytes += entry.size;
                    if entries.len() >= MAX_PER_NOTE as usize || attached_bytes > MAX_STORAGE_BYTES as usize
                        || kind != entry.kind || !has_expected_signature(kind, content)
                    {
                        return Err("附件超出容量或格式不符，匯出已取消。".into());
                    }
                    write_entry(&mut writer, &entry.path(entries.len()), content)?;
                    entries.push(entry);
                }
                count = entries.len();
                let files = entries.iter().map(|entry| json!({ "name": entry.name, "bytes": entry.size }))
                    .collect::<Vec<_>>();
                let manifest = json!({ "format": FORMAT, "title": title, "attachments": files });
                write_entry(&mut writer, MANIFEST, &serde_json::to_vec_pretty(&manifest)?)?;
                write_entry(&mut writer, MARKDOWN, body.as_bytes())?;
                write_entry(&mut writer, INDEX, attachment_index(&entries).as_bytes())?;
                writer.finish()?;
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok(count)
    }

    pub fn import_note_bundle(&mut self, path: &Path) -> Result<(i64, usize)> {
        #[cfg(windows)]
        let (absolute, _parents) = super::pin_local_parents(path)?;
        #[cfg(windows)]
        let path = absolute.as_path();
        let file = super::open_checked(path, false)?;
        if file.metadata()?.len() > MAX_ARCHIVE {
            return Err("紙間筆記與附件 ZIP 超出大小上限。".into());
        }
        let mut archive = ZipArchive::new(file)?;
        if archive.offset() != 0 || archive.len() > MAX_PER_NOTE as usize + 3
            || archive.has_duplicate_names() || archive.has_overlapping_files()?
        {
            return Err("紙間 ZIP 結構不完整或有重複項目。".into());
        }
        archive.validate_header_consistency()?;
        let manifest = read_entry(&mut archive, MANIFEST, MAX_MANIFEST)?;
        let (title, entries) = parse_manifest(&manifest)?;
        if archive.len() != entries.len() + 3 {
            return Err("紙間 ZIP 的成員數與附件清單不符。".into());
        }
        let body = String::from_utf8(read_entry(&mut archive, MARKDOWN, MAX_NOTE_BYTES)?)?;
        validate_body(&body)?;
        if read_entry(&mut archive, INDEX, MAX_INDEX)? != attachment_index(&entries).as_bytes() {
            return Err("紙間 ZIP 的附件索引與清單不符。".into());
        }

        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let used: i64 = tx.query_row("SELECT coalesce(sum(length(content)),0) FROM attachments", [], |row| row.get(0))?;
        let extra: i64 = entries.iter().map(|entry| entry.size as i64).sum();
        if used > MAX_STORAGE_BYTES - extra {
            return Err("工作區附件總量上限為 128 MiB。".into());
        }
        let id = insert(&tx, None, &title, &body)?;
        for (index, entry) in entries.iter().enumerate() {
            let content = read_entry(&mut archive, &entry.path(index), entry.size)?;
            if content.len() != entry.size || !has_expected_signature(entry.kind, &content) {
                return Err("紙間 ZIP 的附件內容與清單不符，匯入已取消。".into());
            }
            tx.execute(
                "INSERT INTO attachments(note_id,name,kind,content) VALUES (?1,?2,?3,?4)",
                params![id, entry.name, entry.kind, content],
            )?;
        }
        tx.commit()?;
        Ok((id, entries.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn note_bundle_preserves_markdown_and_duplicate_attachment_names_across_workspaces() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let source = dir.join("同名.txt");
        let bundle = dir.join("可攜.zip");
        let mut original = Notebook::open(&dir.join("original.sqlite3"), false)?;
        let mut target = Notebook::open(&dir.join("target.sqlite3"), false)?;
        let body = "\u{feff}---\r\n標籤: 中文\r\n---\r\n原文";
        let note = original.add(None, "附件驗收", body)?;
        for content in ["第一份\r\n", "第二份\n"] {
            fs::write(&source, content)?;
            original.import_attachment(note, &source)?;
        }
        assert_eq!(original.export_note_bundle(note, &bundle)?, 2);
        let expected = fs::read(&bundle)?;
        assert!(original.export_note_bundle(note, &bundle).is_err());
        assert_eq!(fs::read(&bundle)?, expected);
        let existing = target.add(None, "附件驗收", "同名筆記留存")?;
        let (new, count) = target.import_note_bundle(&bundle)?;
        assert_ne!(new, existing);
        assert_eq!(count, 2);
        assert_eq!(target.note(new)?.unwrap().body, body);
        assert_eq!(target.note(existing)?.unwrap().body, "同名筆記留存");
        let files = target.list_attachments(new)?;
        assert_eq!(files[0].name, files[1].name);
        for (index, content) in ["第一份\r\n", "第二份\n"].into_iter().enumerate() {
            let output = dir.join(format!("{index}.txt"));
            target.save_attachment_copy(files[index].id, &output)?;
            assert_eq!(fs::read(output)?, content.as_bytes());
        }
        drop(original);
        drop(target);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn malformed_second_attachment_rolls_back_first_insert_and_note() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("拒絕.zip");
        let mut writer = ZipWriter::new(File::create(&path)?);
        let files = [Entry::new("a.txt", 5)?, Entry::new("b.pdf", 5)?];
        let manifest = json!({ "format": FORMAT, "title": "不得留下", "attachments": [
            {"name":"a.txt","bytes":5},{"name":"b.pdf","bytes":5}
        ]});
        write_entry(&mut writer, MANIFEST, &serde_json::to_vec(&manifest)?)?;
        write_entry(&mut writer, MARKDOWN, b"text")?;
        write_entry(&mut writer, INDEX, attachment_index(&files).as_bytes())?;
        write_entry(&mut writer, &files[0].path(0), b"hello")?;
        write_entry(&mut writer, &files[1].path(1), b"wrong")?;
        drop(writer.finish()?);
        assert!(Entry::new("../outside.txt", 5).is_err());
        assert!(Entry::new("AUX.md", 5).is_err());
        let mut db = Notebook::open(&dir.join("target.sqlite3"), false)?;
        assert!(db.import_note_bundle(&path).is_err());
        assert_eq!(db.connection.query_row("SELECT count(*) FROM notes", [], |r| r.get::<_, i64>(0))?, 0);
        assert_eq!(db.connection.query_row("SELECT count(*) FROM attachments", [], |r| r.get::<_, i64>(0))?, 0);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
