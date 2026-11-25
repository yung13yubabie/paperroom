use crate::store::{MAX_NOTE_BYTES, Result, clean_title, migration_media::{
    MediaPlan, referenced_paths, source_relative,
}};
use crate::store::notion_handoff::CsvSources;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
#[cfg(test)]
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

pub(crate) const MAX_ZIP_BYTES: u64 = 120 * 1024 * 1024;
const MAX_TOTAL_TEXT_BYTES: u64 = 80 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 15_000;
const MAX_DEPTH: usize = 64;
// Match the attachment limit before inflating untrusted ZIP data.
const MAX_MEDIA_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct ImportedPage {
    pub parts: Vec<String>,
    pub title: String,
    pub body: String,
}

#[derive(Default)]
pub struct Import {
    pub(crate) notion_members: Vec<(Option<String>, String)>,
    pub pages: Vec<ImportedPage>,
    pub source_paths: BTreeMap<Vec<String>, String>,
    pub skipped: usize,
    has_all_data_json: bool,
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
    let file = File::open(path)?;
    let bytes = read_bounded(file, MAX_NOTE_BYTES as u64)?;
    let text = String::from_utf8(bytes)?;
    if text.contains('\0') {
        return Err("Markdown 不能包含 NUL 字元。".into());
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("匯入筆記");
    Ok(as_page(vec![stem.to_owned()], text))
}

pub fn archive(path: &Path) -> Result<Import> {
    read_archive(path, None, None)
}

pub(crate) fn archive_with_media(path: &Path) -> Result<(Import, MediaPlan)> {
    let mut media = MediaPlan::default();
    let imported = read_archive(path, Some(&mut media), None)?;
    Ok((imported, media))
}

pub(crate) fn notion_archive_with_media(path: &Path) -> Result<(Import, MediaPlan, CsvSources)> {
    let mut media = MediaPlan::default();
    let mut csv = CsvSources::default();
    let imported = read_archive(path, Some(&mut media), Some(&mut csv))?;
    Ok((imported, media, csv))
}

impl Import {
    pub(crate) fn markdown_zip_import_notice(&self) -> Option<&'static str> {
        self.has_all_data_json.then_some(
            "ZIP 含有 All-Data.json，可能是 Heptabase 完整備份。紙間只會匯入 Markdown 與可辨識的附件，不會匯入白板配置、卡片關聯或備份中繼資料。仍要繼續？"
        )
    }
}

fn read_archive(path: &Path, media: Option<&mut MediaPlan>, csv: Option<&mut CsvSources>) -> Result<Import> {
    let snapshot = bounded_zip_snapshot(path)?;
    read_archive_snapshot(&snapshot, media, csv)
}

pub(crate) fn bounded_zip_snapshot(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_ZIP_BYTES {
        return Err("匯入 ZIP 超過 120 MB。".into());
    }
    read_bounded(file, MAX_ZIP_BYTES)
}

pub(crate) fn read_archive_snapshot(
    snapshot: &[u8], media: Option<&mut MediaPlan>, mut csv: Option<&mut CsvSources>,
) -> Result<Import> {
    if snapshot.len() as u64 > MAX_ZIP_BYTES {
        return Err("匯入 ZIP 超過 120 MB。".into());
    }
    let mut result = Import::default();
    collect(snapshot, &mut result, None, media, csv.as_deref_mut())?;
    if result.pages.is_empty() && csv.as_ref().is_none_or(|sources| sources.count() == 0) {
        let error = if csv.is_some() {
            "ZIP 中找不到 UTF-8 Markdown 或 CSV 原檔。"
        } else {
            "ZIP 中找不到 UTF-8 Markdown 檔案。"
        };
        return Err(error.into());
    }
    result
        .pages
        .sort_by(|left, right| left.parts.cmp(&right.parts));
    if result
        .pages
        .windows(2)
        .any(|pair| pair[0].parts == pair[1].parts)
    {
        return Err("這個 ZIP 內有相同路徑的 Markdown。請保留每個路徑的一份筆記。".into());
    }
    Ok(result)
}

pub(crate) fn read_bounded(entry: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    entry.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("匯入項目超過解壓縮容量上限。".into());
    }
    Ok(bytes)
}

fn safe_archive_path(path: &str) -> bool {
    let parts = path.split('/');
    parts.clone().count() <= MAX_DEPTH && parts.clone().all(|part| {
        !matches!(part, "" | "." | ".." | "__MACOSX")
            && !part.contains([':', '\0'])
    })
}

fn collect(
    contents: &[u8],
    out: &mut Import,
    part: Option<&str>,
    mut media: Option<&mut MediaPlan>,
    mut csv: Option<&mut CsvSources>,
) -> Result<()> {
    let mut archive =
        zip::unstable::read::ZipIterable::try_new(Cursor::new(contents), Default::default())?
            .enforce_header_consistency(true);
    if archive.len() > MAX_ENTRIES - out.visited {
        return Err("ZIP 內的項目數量超過 15,000。".into());
    }
    out.visited += archive.len();

    // Each inner ZIP is its own attachment namespace. Never bind a reference
    // in one part to a similarly named entry in a different part.
    let mut paths = BTreeMap::<String, usize>::new();
    let mut wanted = BTreeMap::<String, BTreeSet<usize>>::new();
    let mut references = 0;
    let mut local_notes = Vec::new();
    let mut original = BTreeMap::new();
    let mut bodies = Cursor::new(contents);
    for entry in archive.files()? {
        let entry = entry?;
        if entry.is_dir() {
            continue;
        }
        let raw_path = entry.name()?;
        let path = raw_path.replace('\\', "/");
        let filename = path.rsplit('/').next().unwrap_or(&path);
        let (stem, ext) = filename.rsplit_once('.').unwrap_or((filename, ""));
        let csv_extension = filename.trim_end_matches([' ', '.']).rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("csv"));
        if let Some(sources) = csv.as_deref_mut() && csv_extension {
            let size = entry.size();
            sources.check_member(part, &raw_path, size, entry.is_symlink())?;
            let raw_path = raw_path.into_owned();
            let bytes = read_bounded(entry.with_reader(&mut bodies, Default::default())?, size)?;
            if bytes.len() as u64 != size {
                return Err("Notion CSV 的原檔大小與 ZIP 記錄不符。".into());
            }
            sources.capture(part, &raw_path, bytes)?;
            continue;
        }
        if !safe_archive_path(&path) || entry.is_symlink() {
            out.skipped += 1;
            continue;
        }
        // Only identify the safe, regular member name. Do not read or interpret
        // its body as a native backup, and do not change what Markdown import accepts.
        if filename.eq_ignore_ascii_case("All-Data.json") {
            out.has_all_data_json = true;
        }
        if part.is_none() && !path.contains('/') && ext.eq_ignore_ascii_case("zip") {
            if let Some(sources) = csv.as_deref_mut() {
                sources.add_namespace(&raw_path)?;
            }
            let remaining = MAX_ZIP_BYTES.saturating_sub(out.unpacked_archives);
            if entry.size() > remaining {
                return Err("內嵌 ZIP 的總容量超過 120 MB。".into());
            }
            let contents = read_bounded(
                entry.with_reader(&mut bodies, Default::default())?,
                remaining,
            )?;
            out.unpacked_archives += contents.len() as u64;
            collect(&contents, out, Some(&path), media.as_deref_mut(), csv.as_deref_mut())?;
            continue;
        }
        if !(ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")) {
            if media.is_some() && crate::store::migration_media::supported(Path::new(&path)) {
                *paths.entry(path).or_default() += 1;
            }
            out.skipped += 1;
            continue;
        }
        let original_notion_path = csv.is_some() && raw_path == path;
        let limit = (MAX_TOTAL_TEXT_BYTES - out.total_bytes).min(MAX_NOTE_BYTES as u64);
        if entry.size() > limit {
            return Err("Markdown 匯入上限為單篇 4 MB、合計 80 MB。".into());
        }
        let bytes = read_bounded(entry.with_reader(&mut bodies, Default::default())?, limit)?;
        out.total_bytes += bytes.len() as u64;
        let Ok(contents) = String::from_utf8(bytes) else {
            out.skipped += 1;
            continue;
        };
        if contents.contains('\0') {
            out.skipped += 1;
            continue;
        }
        let source: Vec<String> = path.split('/').map(str::to_owned).collect();
        let mut keys = source.clone();
        if let Some(last) = keys.last_mut() {
            *last = stem.to_owned();
        }
        let page = as_page(keys, contents);
        if let Some(plan) = media.as_deref_mut() {
            let note = out.pages.len();
            local_notes.push((note,path.clone()));
            let mut missing = BTreeSet::new();
            let mut found = BTreeSet::new();
            for (raw, wiki) in referenced_paths(&page.body) {
                if wiki {
                    continue;
                }
                if let Some(relative) = source_relative(&source, &raw) {
                    original.entry((note,relative.clone())).or_insert(raw);
                    found.insert(relative);
                } else {
                    missing.insert(raw);
                }
                if found.len() + missing.len() > MAX_ENTRIES {
                    return Err("筆記內的附件連結超過上限。".into());
                }
            }
            references += found.len() + missing.len();
            if references > MAX_ENTRIES {
                return Err("ZIP 內的附件連結超過上限。".into());
            }
            for raw in &missing {plan.issue(&path,raw,"附件","無效或超出來源範圍的路徑");}
            plan.unresolved += missing.len();
            for path in found {
                wanted.entry(path).or_default().insert(note);
            }
        }
        if original_notion_path {
            out.notion_members.push((part.map(str::to_owned), path.clone()));
        }
        out.source_paths.insert(page.parts.clone(), path);
        out.pages.push(page);
    }

    let Some(plan) = media else { return Ok(()) };
    let notes = local_notes.into_iter().map(|(i,path)|(&out.pages[i],path)).collect::<Vec<_>>();
    plan.check_notes(&notes,false);
    if wanted.is_empty() {
        return Ok(());
    }
    for entry in archive.files()? {
        let entry = entry?;
        if entry.is_dir() || entry.is_symlink() {
            continue;
        }
        let path = entry.name()?.replace('\\', "/");
        if paths.get(&path) != Some(&1) {
            continue;
        }
        let Some(source_notes) = wanted.remove(&path) else {
            continue;
        };
        if entry.size() > MAX_MEDIA_BYTES {
            return Err("ZIP 內的附件超過 8 MiB。".into());
        }
        let bytes = read_bounded(
            entry.with_reader(&mut bodies, Default::default())?,
            MAX_MEDIA_BYTES,
        )?;
        for note in source_notes {
            plan.add_bytes(&out.pages[note].parts.join("/"), &path, &bytes)?;
        }
        out.skipped -= 1;
    }
    for (path,ids) in &wanted {
        let reason=if paths.get(path).copied().unwrap_or(0)>1 {"ZIP 含同路徑的多個附件"} else {"找不到可匯入的來源檔案"};
        for id in ids {
            let source=&out.source_paths[&out.pages[*id].parts];
            plan.issue(source,original.get(&(*id,path.clone())).map_or(path.as_str(),String::as_str),"附件",reason);
        }
    }
    plan.unresolved += wanted.values().map(BTreeSet::len).sum::<usize>();
    Ok(())
}

pub fn export_note(path: &Path, title: &str, body: &str) -> Result<()> {
    use std::io::Write;

    crate::output::publish_new(path, |file, _| {
        if has_frontmatter(body) {
            file.write_all(body.as_bytes())?;
        } else {
            write!(file, "# {}\n\n{}", title, body)?;
        }
        Ok(())
    })
}

fn has_frontmatter(body: &str) -> bool {
    let mut lines = body.trim_start_matches('\u{feff}').lines();
    lines.next() == Some("---") && lines.any(|line| line == "---" || line == "...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Notebook, View};
    use std::io::Write;

    #[test]
    fn exports_frontmatter_at_the_original_start_of_the_file() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-frontmatter-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let path = dir.join("export.md");
        let text = "\u{feff}---\r\nstatus: draft\r\n---\r\n# 原筆記\r\n";
        export_note(&path, "介面標題", text)?;
        assert_eq!(fs::read(&path)?, text.as_bytes());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn text_import_is_bounded_and_rejects_nul_in_the_heading() -> Result<()> {
        assert_eq!(read_bounded(Cursor::new(b"abc"), 3)?, b"abc");
        assert!(read_bounded(Cursor::new(b"abcd"), 3).is_err());
        let dir = std::env::temp_dir().join(format!("paperroom-text-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let path = dir.join("literal.md");
        fs::write(&path, "# 原稿\n\n{\\rtf1}\n中文")?;
        assert_eq!(markdown(&path)?.body, "{\\rtf1}\n中文");
        fs::write(&path, "# a\0b\n\nbody")?;
        assert!(markdown(&path).is_err());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn nested_archive_cannot_silently_replace_existing_markdown() -> Result<()> {
        fn zip_bytes(name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            writer.start_file(name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(bytes)?;
            Ok(writer.finish()?.into_inner())
        }
        let dir = std::env::temp_dir().join(format!("paperroom-parts-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let path = dir.join("export.zip");
        let mut outer = zip::ZipWriter::new(File::create(&path)?);
        for (part, content) in [
            ("Part-1.zip", b"first".as_slice()),
            ("Part-2.zip", b"second".as_slice()),
        ] {
            outer.start_file(part, zip::write::SimpleFileOptions::default())?;
            outer.write_all(&zip_bytes("note.md", content)?)?;
        }
        outer.finish()?;
        assert!(
            archive(&path)
                .err()
                .is_some_and(|error| error.to_string().contains("相同路徑"))
        );
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn synthetic_attachment_report_fixture() -> Result<()> {
        // Generated test data; this exercises Paperroom's real ZIP importer, not Heptabase.
        let dir = std::env::temp_dir().join(format!("paperroom-media-report-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let fixture = dir.join("synthetic-markdown-export.zip");
        let result = (|| -> Result<()> {
            let mut writer = zip::ZipWriter::new(fs::File::create(&fixture)?);
            let md = r#"# 卡片
![封面](media/cover.png)
[文件](media/sample.pdf)
[missing](media/missing.pdf)
[remote](https://example.invalid/cloud.pdf)
[backup](All-Data.json)
"#;
            writer.start_file("卡片.md", zip::write::SimpleFileOptions::default())?;
            writer.write_all(md.as_bytes())?;
            for (name, data) in [
                ("media/cover.png", b"\x89PNG\r\n\x1a\n".as_slice()),
                ("media/sample.pdf", b"%PDF-1.4\n".as_slice()),
                ("All-Data.json", b"{}".as_slice()),
            ] {
                writer.start_file(name, zip::write::SimpleFileOptions::default())?;
                writer.write_all(data)?;
            }
            writer.finish()?;
            let (notes, plan) = archive_with_media(&fixture)?;
            let report = plan.report(notes.pages.len(), notes.skipped);
            assert_eq!((notes.pages.len(), notes.skipped, plan.count(), plan.unresolved), (1, 1, 2, 1));
            assert!(report.contains("卡片.md　附件　media/missing.pdf　找不到"));
            assert!(!report.contains("cloud.pdf") && !report.contains("All-Data.json"));
            let notice = notes.markdown_zip_import_notice().ok_or("expected backup marker notice")?;
            assert!(notice.contains("All-Data.json") && notice.contains("不會匯入白板配置"));
            Ok(())
        })();
        fs::remove_dir_all(&dir)?;
        result
    }

    #[test]
    fn synthetic_markdown_zip_backup_notice_requires_safe_exact_filename() -> Result<()> {
        fn zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for (path, content) in entries {
                writer.start_file(*path, zip::write::SimpleFileOptions::default())?;
                writer.write_all(content)?;
            }
            Ok(writer.finish()?.into_inner())
        }

        // This deliberately is not valid JSON: only the nested member name should matter.
        let notes = b"# card\n\nText.";
        let part = zip(&[("Backup/card.md", notes), ("Backup/All-Data.json", b"not JSON")])?;
        let archive = zip(&[("Part.zip", &part)])?;
        let imported = read_archive_snapshot(&archive, Some(&mut MediaPlan::default()), None)?;
        assert_eq!((imported.pages.len(), imported.skipped), (1, 1));
        assert!(imported.markdown_zip_import_notice().is_some());

        let ordinary = zip(&[("card.md", notes), ("All-Data.json.bak", b"not JSON"), ("../All-Data.json", b"unsafe path")])?;
        let imported = read_archive_snapshot(&ordinary, Some(&mut MediaPlan::default()), None)?;
        assert_eq!((imported.pages.len(), imported.skipped), (1, 2));
        assert!(imported.markdown_zip_import_notice().is_none());
        Ok(())
    }

    #[test]
    fn markdown_zip_preserves_exact_relative_attachments_within_each_part() -> Result<()> {
        fn zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for (path, content) in entries {
                writer.start_file(*path, zip::write::SimpleFileOptions::default())?;
                writer.write_all(content)?;
            }
            Ok(writer.finish()?.into_inner())
        }

        let dir = std::env::temp_dir().join(format!("paperroom-zip-media-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let path = dir.join("markdown-export.zip");
        let notion = zip(&[
            (
                "Notion Page.md",
                b"# First\n\n[PDF](Notion%20Page/paper.pdf)\n[missing](Notion%20Page/missing.txt)\n[other part](Notes/research/paper.pdf)\n[external](https://example.test/a.pdf)\n",
            ),
            ("Notion Page/paper.pdf", b"%PDF-1.7\nFirst, exact bytes.\n"),
            ("another/missing.txt", b"Same basename cannot be matched."),
        ])?;
        let generic = zip(&[
            (
                "Notes/Research.markdown",
                b"# Second\n\n[PDF](research/paper.pdf)\n![Photo](research/photo.png)\n[Text](../shared.txt#details)",
            ),
            ("Notes/research/paper.pdf", b"%PDF-1.7\nOther PDF, same basename.\n"),
            ("Notes/research/photo.png", b"\x89PNG\r\n\x1a\nPNG-signature fixture"),
            ("shared.txt", b"Local UTF-8 contents."),
        ])?;
        fs::write(&path, zip(&[("Part-1.zip", &notion), ("Part-2.zip", &generic)])?)?;

        let (imported, plan) = archive_with_media(&path)?;
        assert_eq!(imported.pages.len(), 2);
        assert_eq!(imported.source_paths[&vec!["Notion Page".to_owned()]], "Notion Page.md");
        assert_eq!(
            imported.source_paths[&vec!["Notes".to_owned(), "Research".to_owned()]],
            "Notes/Research.markdown"
        );
        assert_eq!(plan.count(), 4);
        assert_eq!(plan.unresolved, 2);
        let report=plan.report(imported.pages.len(),imported.skipped);
        assert!(report.contains("Notion Page.md　附件　Notion%20Page/missing.txt　找不到"));
        assert!(!report.contains("example.test"));
        assert_eq!(imported.skipped, 1);

        let mut notebook = Notebook::open(&dir.join("import.sqlite3"), false)?;
        notebook.import_markdown_with_media("Archive", imported.pages, plan)?;
        for (query, origin, expected, expected_count) in [
            ("First", "Notion Page/paper.pdf", b"%PDF-1.7\nFirst, exact bytes.\n".as_slice(), 1),
            ("Second", "Notes/research/paper.pdf", b"%PDF-1.7\nOther PDF, same basename.\n".as_slice(), 3),
        ] {
            let note = notebook.rows(View::All, query)?[0].id;
            let attachments = notebook.list_attachments(note)?;
            assert_eq!(attachments.len(), expected_count);
            let pdf = attachments.iter().find(|attachment| attachment.name == "paper.pdf").unwrap();
            assert_eq!(notebook.attachment_origin(pdf.id)?.as_deref(), Some(origin));
            assert_eq!(notebook.read_pdf_attachment(pdf.id)?.1, expected);
        }
        drop(notebook);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

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
