use crate::store::{MAX_NOTE_BYTES, Result, clean_title};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Cursor, Read, Seek};
use std::path::Path;

const MAX_ZIP_BYTES: u64 = 120 * 1024 * 1024;
const MAX_TOTAL_TEXT_BYTES: u64 = 80 * 1024 * 1024;
const MAX_ENTRIES: usize = 15_000;
const MAX_DEPTH: usize = 64;

#[derive(Debug)]
pub struct ImportedPage {
    pub parts: Vec<String>,
    pub title: String,
    pub body: String,
}

#[derive(Default)]
pub struct Import {
    pub pages: Vec<ImportedPage>,
    pub skipped: usize,
    visited: usize,
    total_bytes: u64,
    unpacked_archives: u64,
}

pub fn export_title(stem: &str) -> String {
    if let Some((name, suffix)) = stem.rsplit_once(' ')
        && suffix.len() == 32
        && suffix.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return clean_title(name);
    }
    clean_title(stem)
}

fn as_page(parts: Vec<String>, contents: String) -> ImportedPage {
    let contents = contents.trim_start_matches('\u{feff}');
    let fallback = export_title(parts.last().map_or("", String::as_str));
    let (title, body) = match contents.split_once('\n') {
        Some((first, rest)) if first.starts_with("# ") => (
            clean_title(&first[2..]),
            rest.strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
                .unwrap_or(rest),
        ),
        None if contents.starts_with("# ") => (clean_title(&contents[2..]), ""),
        _ => (fallback, contents),
    };
    ImportedPage {
        parts,
        title,
        body: body.replace("\r\n", "\n"),
    }
}

pub fn markdown(path: &Path) -> Result<ImportedPage> {
    if fs::metadata(path)?.len() > MAX_NOTE_BYTES as u64 {
        return Err("單篇 Markdown 超過 4 MB。".into());
    }
    let text = fs::read_to_string(path)?;
    if text.len() > MAX_NOTE_BYTES {
        return Err("單篇 Markdown 超過 4 MB。".into());
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("匯入筆記");
    Ok(as_page(vec![stem.to_owned()], text))
}

pub fn archive(path: &Path) -> Result<Import> {
    if fs::metadata(path)?.len() > MAX_ZIP_BYTES {
        return Err("匯入 ZIP 超過 120 MB。".into());
    }
    let mut result = Import::default();
    collect(File::open(path)?, &mut result, true)?;
    if result.pages.is_empty() {
        return Err("ZIP 中找不到 UTF-8 Markdown 檔案。".into());
    }
    let mut paths = HashSet::new();
    if result.pages.iter().any(|p| !paths.insert(p.parts.clone())) {
        return Err("這個 ZIP 內有相同路徑的 Markdown。請保留每個路徑的一份筆記。".into());
    }
    result
        .pages
        .sort_by(|left, right| left.parts.cmp(&right.parts));
    Ok(result)
}

fn read_bounded(entry: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    entry.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("匯入項目超過解壓縮容量上限。".into());
    }
    Ok(bytes)
}

fn collect(reader: impl Read + Seek, out: &mut Import, outer: bool) -> Result<()> {
    let mut archive = zip::ZipArchive::new(reader)?;
    out.visited += archive.len();
    if out.visited > MAX_ENTRIES {
        return Err("ZIP 內的項目數量超過 15,000。".into());
    }
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let path = entry.name().replace('\\', "/");
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() > MAX_DEPTH
            || parts.iter().any(|p| {
                matches!(*p, "" | "." | ".." | "__MACOSX") || p.contains(':') || p.contains('\0')
            })
            || entry.is_symlink()
        {
            out.skipped += 1;
            continue;
        }
        let filename = parts[parts.len() - 1];
        let (stem, ext) = filename.rsplit_once('.').unwrap_or((filename, ""));
        if outer && parts.len() == 1 && ext.eq_ignore_ascii_case("zip") {
            let remaining = MAX_ZIP_BYTES.saturating_sub(out.unpacked_archives);
            if entry.size() > remaining {
                return Err("內嵌 ZIP 的總容量超過 120 MB。".into());
            }
            let contents = read_bounded(entry, remaining)?;
            out.unpacked_archives += contents.len() as u64;
            collect(Cursor::new(contents), out, false)?;
            continue;
        }
        if !(ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")) {
            out.skipped += 1;
            continue;
        }
        let limit = (MAX_TOTAL_TEXT_BYTES - out.total_bytes).min(MAX_NOTE_BYTES as u64);
        if entry.size() > limit {
            return Err("Markdown 匯入上限為單篇 4 MB、合計 80 MB。".into());
        }
        let bytes = read_bounded(entry, limit)?;
        out.total_bytes += bytes.len() as u64;
        let Ok(contents) = String::from_utf8(bytes) else {
            out.skipped += 1;
            continue;
        };
        if contents.contains('\0') {
            out.skipped += 1;
            continue;
        }
        let mut keys = parts[..parts.len() - 1]
            .iter()
            .map(|part| (*part).to_owned())
            .collect::<Vec<_>>();
        keys.push(stem.to_owned());
        out.pages.push(as_page(keys, contents));
    }
    Ok(())
}

pub fn export_note(path: &Path, title: &str, body: &str) -> Result<()> {
    use std::io::Write;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let written = write!(file, "# {}\n\n{}", title, body).and_then(|_| file.sync_all());
    if let Err(error) = written {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Notebook, View};
    use std::io::Write;

    #[test]
    fn notion_archive_retains_parent_child_and_ignores_non_notes() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-archive-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let path = dir.join("tests.zip");
        let parent = "研究 c5205b396d29440fac4cf01a11060e79";
        let mut archive_writer = zip::ZipWriter::new(File::create(&path)?);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in [
            (format!("{parent}.md"), "# 研究\r\n\r\n父頁"),
            (format!("{parent}/中文子頁.md"), "# 中文子頁\n\n- [ ] 路線"),
            (format!("{parent}/匯出.csv"), "欄位,狀態"),
            ("../escape.md".to_owned(), "no"),
        ] {
            archive_writer.start_file(name, options)?;
            archive_writer.write_all(content.as_bytes())?;
        }
        archive_writer.finish()?;
        let imported = archive(&path)?;
        assert_eq!(imported.pages.len(), 2);
        assert_eq!(imported.skipped, 2);
        assert_eq!(export_title(parent), "研究");
        let mut notebook = Notebook::open(&dir.join("import.sqlite3"), false)?;
        let root = notebook.import("匯入", imported.pages)?;
        let rows = notebook.rows(View::All, "")?;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].parent, Some(root));
        assert_eq!(rows[2].parent, Some(rows[1].id));
        assert_eq!(notebook.note(rows[1].id)?.unwrap().body, "父頁");
        assert_eq!(notebook.rows(View::All, "路線")?[0].title, "中文子頁");
        let export = dir.join("output.md");
        let note = notebook.note(rows[2].id)?.unwrap();
        export_note(&export, &note.row.title, &note.body)?;
        assert!(export_note(&export, "勿覆蓋", "新").is_err());
        assert_eq!(markdown(&export)?.body, "- [ ] 路線");
        drop(notebook);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
