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
    pub pages: Vec<ImportedPage>,
    pub source_paths: BTreeMap<Vec<String>, String>,
    pub skipped: usize,
    has_all_data_json: bool,
    pub(crate) has_mcp_transfer_manifest: bool,
    visited: usize,
    total_bytes: u64,
    unpacked_archives: u64,
    companion_canvases: Vec<(String,String)>,
    companion_warnings: Vec<String>,
}

pub(crate) struct HeptabaseHandoff {
    pub paths: BTreeMap<Vec<String>,String>,
    pub canvases: Vec<(String,String)>,
    pub warnings: Vec<String>,
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
    let snapshot = bounded_zip_snapshot(path)?;
    let mut imported = read_archive_snapshot(&snapshot, Some(&mut media), None)?;
    if imported.is_flat_heptabase_backup() {
        imported.read_canvas_companions(&snapshot)?;
    }
    Ok((imported, media))
}

pub(crate) fn notion_archive_with_media(path: &Path) -> Result<(Import, MediaPlan, CsvSources)> {
    let mut media = MediaPlan::default();
    let mut csv = CsvSources::default();
    let imported = read_archive(path, Some(&mut media), Some(&mut csv))?;
    if let Some(plan) = &csv.typed_relations
        && (!imported.has_mcp_transfer_manifest || plan.page_count() != imported.pages.len() || csv.count() != 0 || media.file_count() != 0) {
        return Err("紙間具名關聯交接標記或確認頁數不符，或會另匯入 CSV 或附件。未執行匯入。".into());
    }
    Ok((imported, media, csv))
}

impl Import {
    fn is_flat_heptabase_backup(&self) -> bool {
        self.has_all_data_json && self.unpacked_archives == 0
    }

    pub(crate) fn take_heptabase_handoff(&mut self) -> Option<HeptabaseHandoff> {
        self.is_flat_heptabase_backup().then(|| HeptabaseHandoff {
            paths: std::mem::take(&mut self.source_paths),
            canvases: std::mem::take(&mut self.companion_canvases),
            warnings: std::mem::take(&mut self.companion_warnings),
        })
    }

    pub(crate) fn markdown_zip_import_notice(&self) -> Option<&'static str> {
        if self.is_flat_heptabase_backup() {
            Some("ZIP 含有 All-Data.json，可能是 Heptabase 備份。紙間會匯入 Markdown、可比對的相對附件和同包筆記連結。另外提供的標準 JSON Canvas 若能對應會轉成紙間白板，其餘可讀取的圖將保存為可編輯 Canvas，僅對應原始檔案節點及同包完整路徑。不會讀取 All-Data.json 或還原其中的白板配置、關聯或中繼資料。仍要繼續？")
        } else {
            self.has_all_data_json.then_some(
                "ZIP 含有 All-Data.json 及內嵌 ZIP。紙間只會匯入 Markdown 與可辨識的附件，不會跨 ZIP 配對筆記或白板。仍要繼續？"
            )
        }
    }

    fn read_canvas_companions(&mut self, snapshot: &[u8]) -> Result<()> {
        const MAX_CANVASES: usize = 16;
        const MAX_TOTAL_CANVAS_BYTES: u64 = 16 * 1024 * 1024;

        let mut archive = zip::unstable::read::ZipIterable::try_new(Cursor::new(snapshot), Default::default())?
            .enforce_header_consistency(true);
        let mut bodies = Cursor::new(snapshot);
        let mut documents = BTreeMap::new();
        let mut total = 0;
        for entry in archive.files()? {
            let entry = entry?;
            let name = entry.name()?;
            if entry.is_dir() || entry.is_symlink() || !safe_archive_path(&name)
                || name.contains('\\') || name.chars().any(char::is_control)
                || !Path::new(name.as_ref()).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("canvas")) {
                continue;
            }
            let name = name.into_owned();
            if let Some(previous) = documents.get_mut(&name) {
                *previous = None;
                continue;
            }
            if documents.len() >= MAX_CANVASES {
                if !self.companion_warnings.iter().any(|warning| warning == "只檢查前 16 份 JSON Canvas。") {
                    self.companion_warnings.push("只檢查前 16 份 JSON Canvas。".into());
                }
                continue;
            }
            let limit = MAX_TOTAL_CANVAS_BYTES.saturating_sub(total).min(crate::canvas::MAX_BYTES as u64);
            let size = entry.size();
            let content = if size <= limit {
                let bytes = read_bounded(entry.with_reader(&mut bodies, Default::default())?, limit)?;
                total += bytes.len() as u64;
                String::from_utf8(bytes).ok()
            } else {
                None
            };
            documents.insert(name, content);
        }
        for (name, content) in documents {
            if let Some(content) = content {
                self.companion_canvases.push((name, content));
                self.skipped -= 1;
            } else {
                let displayed: String = name.chars().take(180).collect();
                self.companion_warnings.push(format!("{displayed}：路徑重複、超過容量或不是 UTF-8，未匯入白板。"));
            }
        }
        Ok(())
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
        if part.is_none() && raw_path == crate::store::notion_typed_relations::MEMBER
            && let Some(sources) = csv.as_deref_mut() {
            use crate::store::notion_typed_relations::{LIMIT, Plan};
            if entry.is_symlink() || entry.size() > LIMIT || sources.typed_relations.is_some() {
                return Err("紙間具名關聯交接檔無效、重複或過大。".into());
            }
            let bytes = read_bounded(entry.with_reader(&mut bodies, Default::default())?, LIMIT)?;
            sources.typed_relations = Some(Plan::read(&bytes)?);
            continue;
        }
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
        if part.is_none() && raw_path == "paperroom-transfer/manifest.json" {
            out.has_mcp_transfer_manifest = true;
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
        let original_notion_path = raw_path == path;
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
        if original_notion_path && let Some(sources) = csv.as_deref_mut() {
            sources.record_page_origin(&page.parts, part, &path);
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
    fn heptabase_handoff_keeps_shared_note_ids_and_companion_boards_on_reopen() -> Result<()> {
        use serde_json::json;
        use crate::store::{Notebook, View};

        let dir = std::env::temp_dir().join(format!("paperroom-heptabase-handoff-{}", std::process::id()));
        fs::create_dir(&dir)?;
        let zip_path = dir.join("handoff.zip");
        let workspace = dir.join("workspace.sqlite3");
        let backup = dir.join("restored.sqlite3");
        let mut zip = zip::ZipWriter::new(File::create(&zip_path)?);
        let drawing = |x: i32| json!({"nodes":[
            {"id":"a","type":"file","file":"甲/鏡頭.md","x":x,"y":20,"width":232,"height":114},
            {"id":"b","type":"file","file":"乙/鏡頭.md","x":300,"y":180,"width":232,"height":114}
        ], "edges":[{"id":"ab","fromNode":"a","toNode":"b","toEnd":"none"}]});
        let canvas_a = drawing(10).to_string();
        let canvas_b = drawing(50).to_string();
        let mut wrong = drawing(10);
        wrong["nodes"][0]["file"] = "鏡頭.md".into();
        let wrong = wrong.to_string();
        let mut rich = drawing(-90);
        rich["nodes"][0]["file"] = "乙/鏡頭.md".into();
        rich["nodes"][0]["subpath"] = "#取景".into();
        rich["nodes"][0]["custom"] = json!({"保留":true});
        rich["nodes"].as_array_mut().unwrap().insert(0,json!({"id":"g","type":"group","label":"全景","x":-240,"y":-90,"width":900,"height":550}));
        rich["edges"][0]["toEnd"] = "arrow".into();
        rich["edges"][0]["fromSide"] = "right".into();
        rich["edges"][0]["label"] = "先後".into();
        let raw = format!("{}\r\n", serde_json::to_string_pretty(&rich)?);
        let a_body = "甲正文。\n[乙卡](../乙/鏡頭.md)\n[附件](../媒體/來源.txt)\n[缺件](根目錄候選.md) [[根目錄候選]]";
        for (name, contents) in [
            ("甲/鏡頭.md", format!("# 鏡頭\n\n{a_body}")),
            ("乙/鏡頭.md", "# 鏡頭\n\n乙正文。".into()),
            ("根目錄候選.md", "這篇在根目錄，不能用作缺件的替代。".into()),
            ("媒體/來源.txt", "原始附件".into()),
            ("All-Data.json", "未讀取的原廠內容".into()),
            ("白板/分鏡.canvas", canvas_a),
            ("白板/參考.canvas", canvas_b),
            ("白板/名稱無法確認.canvas", wrong),
            ("白板/構圖.canvas", raw.clone()),
            ("白板/壞圖.canvas", "{bad".into()),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())?;
            zip.write_all(contents.as_bytes())?;
        }
        drop(zip.finish()?);
        let original_zip = fs::read(&zip_path)?;
        let (mut imported, media) = archive_with_media(&zip_path)?;
        let handoff = imported.take_heptabase_handoff().ok_or("expected identified export")?;
        let mut db = Notebook::open(&workspace, false)?;
        let (root, drawings) = db.import_heptabase_handoff("工作區", imported.pages, media, handoff)?;
        assert_eq!(drawings.boards, 2);
        assert_eq!((drawings.canvases, drawings.skipped), (2,1));
        assert_eq!(drawings.details.len(), 3);
        assert_eq!(db.saved_canvases()?.len(), 2);
        let (id, _) = db.saved_canvases()?.into_iter().find(|(_, title)| title=="白板/構圖.canvas").unwrap();
        assert!(db.canvas_vault(id)?.is_none());
        let before = db.read_canvas(id)?;
        assert_eq!(before.as_str(), raw);
        let output = dir.join("已匯出.canvas");
        crate::canvas::export(&output, &before)?;
        assert_eq!(fs::read(&output)?, raw.as_bytes());
        let after = db.edit_canvas(id, &before, crate::store::canvas_edit::CanvasEdit::Geometry {id:"a",geometry:[-99,88,310,190]})?;
        for (key,value) in [("x",-99),("y",88),("width",310),("height",190)] { rich["nodes"][1][key] = value.into(); }
        assert_eq!(after.json(), &rich);
        let inspect = rusqlite::Connection::open_with_flags(&workspace, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let find = |path: &str| inspect.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            (root, path), |row| row.get::<_,i64>(0),
        );
        let (a, b) = (find("甲/鏡頭.md")?, find("乙/鏡頭.md")?);
        drop(inspect);
        assert_ne!(a, b);
        let unresolved = db.saved_canvases()?.into_iter().find(|(_,t)| t.ends_with("名稱無法確認.canvas")).unwrap().0;
        assert_eq!(db.resolve_canvas_file_link(unresolved,None,"a")?.note_id,None);
        assert_eq!(db.resolve_canvas_file_link(unresolved,None,"b")?.note_id,Some(b));
        assert!(db.set_canvas_vault(id, Some(root)).is_err());
        let a_note = db.note(a)?.unwrap();
        assert_eq!(a_note.body, a_body);
        assert_eq!(db.note(b)?.unwrap().body, "乙正文。");
        assert_eq!(db.rows(View::All,"鏡頭")?.len(), 2);
        assert_eq!(db.vault_note_links(&a_note)?[0].note_id, b);
        assert_eq!(db.graph_neighbors(&db.note(b)?.unwrap())?.1[0].0, a);
        db.save(a, a_note.version, "甲已改名", a_body)?;
        db.backup(&backup)?;
        drop(db);

        for path in [&workspace, &backup] {
            let reopened = Notebook::open(path, false)?;
            assert!(reopened.canvas_vault(id)?.is_none());
            assert_eq!(reopened.read_canvas(id)?.json(), &rich);
            let (_, links) = reopened.canvas_file_link_snapshot(id)?;
            assert_eq!(links.iter().map(|l| l.note_id).collect::<Vec<_>>(), [Some(b),Some(b)]);
            let placement = reopened.canvas_note_placements(b)?.into_iter().find(|p| p.canvas_id==id).unwrap();
            let hits = reopened.canvas_note_occurrences(b,&placement)?;
            assert_eq!(hits.len(),2);
            reopened.locate_canvas_note(b,&placement,&hits[0])?;
            if path==&backup {
                use crate::store::canvas_edit::CanvasEdit;
                let before = reopened.read_canvas(id)?;
                let changed = reopened.edit_canvas(id,&before,CanvasEdit::Content{id:"a",value:"甲/鏡頭.md"})?;
                assert_eq!(reopened.resolve_canvas_file_link(id,None,"a")?.note_id,None);
                assert!(reopened.locate_canvas_note(b,&placement,&hits[0]).is_err());
                let changed = reopened.edit_canvas(id,&changed,CanvasEdit::Content{id:"a",value:"乙/./鏡頭.md"})?;
                assert_eq!(reopened.resolve_canvas_file_link(id,None,"a")?.note_id,None);
                reopened.edit_canvas(id,&changed,CanvasEdit::AddNode{kind:"file",content:"乙/鏡頭.md",geometry:[1,2,30,40]})?;
                assert_eq!(reopened.canvas_file_links(id)?.iter().filter(|l|l.note_id.is_some()).count(),1);
                assert_eq!(reopened.canvas_note_occurrences(b,&placement)?.len(),1);
            }
            let readback = rusqlite::Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let check = readback.prepare("SELECT b.title,c.x,c.y FROM board_cards c
                 JOIN boards b ON b.id=c.board_id WHERE c.note_id=?1 ORDER BY b.title")?
                .query_map([a], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i32>(1)?,row.get::<_,i32>(2)?)))?
                .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()?;
            assert_eq!(check, [("分鏡".into(),10,20),("參考".into(),50,20)].into_iter().collect());
            let relations: i64 = readback.query_row(
                "SELECT count(*) FROM board_edges WHERE from_note=?1 AND to_note=?2",
                (a.min(b), a.max(b)), |row| row.get(0),
            )?;
            assert_eq!(relations, 2);
            let attachment: Vec<u8> = readback.query_row(
                "SELECT content FROM attachments WHERE note_id=?1 AND name='來源.txt'",
                [a], |row| row.get(0),
            )?;
            assert_eq!(attachment, "原始附件".as_bytes());
            assert_eq!(reopened.note(a)?.unwrap().row.title, "甲已改名");
            assert_eq!(reopened.note(b)?.unwrap().body, "乙正文。");
        }
        let full = dir.join("quota.sqlite3");
        let mut tight = Notebook::open(&full,false)?;
        let empty = crate::canvas::parse("{}".to_owned())?;
        for _ in 0..127 { tight.save_canvas("既有", &empty)?; }
        let (mut pages, media) = archive_with_media(&zip_path)?;
        let handoff = pages.take_heptabase_handoff().unwrap();
        let old_rows = tight.rows(View::All, "")?.len();
        assert!(tight.import_heptabase_handoff("超額匯入", pages.pages, media, handoff).is_err());
        assert_eq!(tight.saved_canvases()?.len(),127);
        assert_eq!(tight.rows(View::All, "")?.len(), old_rows);
        assert_eq!(fs::read(&zip_path)?, original_zip);
        drop(tight);
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
            assert!(notice.contains("All-Data.json") && notice.contains("還原其中的白板配置"));
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
