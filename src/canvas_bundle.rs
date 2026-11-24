//! Bounded, inert JSON Canvas ZIP transport; no unpacking, execution, or network.
use super::{Notebook, Result, clean_title, workspace::MAX_CANVAS_STORAGE_BYTES};
use crate::{canvas, output::publish_new};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub(super) const SCHEMA: &str = "
CREATE TABLE canvas_bundles(
    canvas_id INTEGER PRIMARY KEY REFERENCES saved_canvases(id) ON DELETE CASCADE,
    canvas_path TEXT NOT NULL,
    inventory TEXT NOT NULL
);
CREATE TABLE canvas_bundle_assets(
    canvas_id INTEGER NOT NULL REFERENCES canvas_bundles(canvas_id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    bytes BLOB NOT NULL,
    PRIMARY KEY(canvas_id,path)
);";

const MAX_ENTRIES: usize = 1024;
const MAX_ARCHIVE: u64 = 132 * 1024 * 1024;
const MAX_ASSET: usize = 16 * 1024 * 1024;
const MAX_CONTENT: usize = 128 * 1024 * 1024;
const TOTAL_ASSETS: i64 = 256 * 1024 * 1024;
const MAX_PREVIEWS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CanvasAssetState { Stored(String), Missing, Ambiguous, Unsafe }

pub(crate) struct CanvasBundleImage {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) bgra: Vec<u8>,
}

pub(crate) struct CanvasBundleView {
    pub(crate) references: BTreeMap<String, CanvasAssetState>,
    pub(crate) previews: BTreeMap<String, CanvasBundleImage>,
    pub(crate) asset_paths: Vec<String>,
}

pub(crate) struct CanvasBundleReport {
    pub(crate) references: BTreeMap<String, CanvasAssetState>,
    pub(crate) unknown_nodes: usize,
}
impl CanvasBundleReport {
    pub(crate) fn summary(&self) -> String {
        let mut text = format!("只保存可精確配對的附件；原始畫布資料照存。\r\n附件參照：{}；未知節點：{}（原始資料保留）。\r\n", self.references.len(), self.unknown_nodes);
        for (path, state) in self.references.iter().take(100) {
            let shown = path.chars().take(200).map(|c| if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { ' ' } else { c }).collect::<String>();
            let status = match state {
                CanvasAssetState::Stored(_) => "已保存", CanvasAssetState::Missing => "缺少",
                CanvasAssetState::Ambiguous => "歧義", CanvasAssetState::Unsafe => "不安全或不在封裝內",
            };
            text.push_str(&format!("\r\n{status}：{shown}"));
        }
        if self.references.len() > 100 { text.push_str("\r\n只列出前 100 筆。"); }
        text
    }
}
pub(crate) struct ImportedCanvasBundle {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) report: CanvasBundleReport,
}

fn portable(path: &str) -> bool {
    path.len() <= 4096 && path.bytes().filter(|b| *b == b'/').count() < 48 && path.split('/').all(|part| {
        let stem = part.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
        let port = stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT"));
        !part.is_empty() && !matches!(part, "." | "..") && part.encode_utf16().count() <= 255
            && !part.ends_with(['.', ' ']) && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
            && !matches!(port, Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
            && !part.chars().any(|c| c.is_control() || r#"<>:"\|?*"#.contains(c))
    })
}

fn canvas_name(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("canvas"))
}

type PathIndex<'a> = BTreeMap<String, Vec<&'a String>>;

fn path_index(names: &BTreeSet<String>) -> PathIndex<'_> {
    let mut index = BTreeMap::<String, Vec<&String>>::new();
    for name in names { index.entry(name.to_lowercase()).or_default().push(name); }
    index
}

fn resolve_indexed(raw: &str, canvas_path: &str, index: &PathIndex<'_>) -> CanvasAssetState {
    if !portable(raw) { return CanvasAssetState::Unsafe; }
    // Try only the vault root, canvas parent, and between them in case of
    // wrapper folders. Never guess by basename, case, or read disk.
    let mut parent = canvas_path;
    let mut expected = BTreeSet::from([raw.to_owned()]);
    while let Some((dir, _)) = parent.rsplit_once('/') {
        expected.insert(format!("{dir}/{raw}"));
        parent = dir;
    }
    let mut hits = BTreeSet::new();
    for name in &expected {
        if let Some(bucket) = index.get(&name.to_lowercase()) {
            for hit in bucket { hits.insert(*hit); if hits.len() > 1 { return CanvasAssetState::Ambiguous; } }
        }
    }
    match hits.into_iter().next() {
        Some(hit) if expected.contains(hit.as_str()) => CanvasAssetState::Stored(hit.clone()),
        _ => CanvasAssetState::Missing,
    }
}

#[cfg(test)]
fn resolve_reference(raw: &str, canvas_path: &str, names: &BTreeSet<String>) -> CanvasAssetState {
    resolve_indexed(raw, canvas_path, &path_index(names))
}
fn report(doc: &canvas::Document, canvas_path: &str, names: &BTreeSet<String>) -> CanvasBundleReport {
    let mut references = BTreeMap::new();
    let mut unknown_nodes = 0;
    let index = path_index(names);
    for node in doc.json()["nodes"].as_array().into_iter().flatten() {
        let key = match node["type"].as_str() {
            Some("file") => node["file"].as_str(),
            Some("group") => node["background"].as_str(),
            Some("text" | "link") => None,
            _ => { unknown_nodes += 1; None }
        };
        if let Some(raw) = key {
            references.entry(raw.into()).or_insert_with(|| resolve_indexed(raw, canvas_path, &index));
        }
    }
    CanvasBundleReport { references, unknown_nodes }
}

struct Input { archive: ZipArchive<File>, sizes: BTreeMap<String, usize>, _parents: Vec<File> }

// Check the raw central-directory count before trusting ZipArchive's indexed
// name map, which can collapse identical names. Support regular ZIP comments.
fn zip_directory(file: &mut File, len: u64) -> Result<(usize, u64)> {
    let tail_len = len.min(22 + u64::from(u16::MAX)) as usize;
    let mut tail = vec![0u8; tail_len];
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    file.read_exact(&mut tail)?;
    for i in (0..=tail.len() - 22).rev() {
        if &tail[i..i + 4] != b"PK\x05\x06" { continue; }
        let end = &tail[i..i + 22];
        let comment = usize::from(u16::from_le_bytes([end[20], end[21]]));
        if i + 22 + comment != tail.len() { continue; }
        let count = usize::from(u16::from_le_bytes([end[10], end[11]]));
        let size = u64::from(u32::from_le_bytes(end[12..16].try_into()?));
        let start = u64::from(u32::from_le_bytes(end[16..20].try_into()?));
        if end[4..8] != [0; 4] || end[8..10] != end[10..12] || !(1..=MAX_ENTRIES).contains(&count)
            || start + size != len - tail_len as u64 + i as u64 {
            return Err("Canvas ZIP 的中央目錄格式不符。".into());
        }
        return Ok((count, start));
    }
    Err("找不到有效的 Canvas ZIP 結尾。".into())
}

fn open_input(path: &Path) -> Result<Input> {
    let (mut file, parents) = super::attachments::open_local_file(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || !(22..=MAX_ARCHIVE).contains(&meta.len()) {
        return Err("Canvas ZIP 超過大小上限或格式不符。".into());
    }
    let (count, start) = zip_directory(&mut file, meta.len())?;
    file.rewind()?;
    let mut archive = ZipArchive::new(file)?;
    if archive.len() != count || archive.offset() != 0 || archive.central_directory_start() != start
        || archive.has_overlapping_entries()? {
        return Err("Canvas ZIP 含重複、交疊或無法辨識的項目。".into());
    }
    let mut sizes = BTreeMap::new();
    let mut used = 0usize;
    for i in 0..count {
        let entry = archive.by_index(i)?;
        let name = entry.name()?;
        if entry.name_raw() != name.as_bytes() || entry.is_symlink() || entry.encrypted()
            || !matches!(entry.compression(), CompressionMethod::Stored | CompressionMethod::Deflated) {
            return Err("Canvas ZIP 含非標準項目、符號連結或加密項目。".into());
        }
        if entry.is_dir() {
            if entry.size() != 0 || !portable(name.trim_end_matches('/'))
                || !name.ends_with('/') || name.ends_with("//")
                || entry.unix_mode().is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o040000))
            {
                return Err("Canvas ZIP 含無效的資料夾路徑。".into());
            }
            continue;
        }
        if !portable(&name) || !entry.is_file()
            || entry.unix_mode().is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000)) {
            return Err("Canvas ZIP 含無法安全讀取的檔案路徑。".into());
        }
        let size = usize::try_from(entry.size())?;
        let limit = if canvas_name(&name) { canvas::MAX_BYTES } else { MAX_ASSET };
        if size > limit || sizes.insert(name.to_string(), size).is_some() {
            return Err("Canvas ZIP 有重複項目或單檔超過大小上限。".into());
        }
        used = used.checked_add(size).filter(|n| *n <= MAX_CONTENT)
            .ok_or("Canvas ZIP 的未壓縮內容超過 128 MiB。")?;
    }
    for name in sizes.keys() {
        let mut component = name.as_str();
        while let Some((parent, _)) = component.rsplit_once('/') {
            if sizes.contains_key(parent) { return Err("Canvas ZIP 的檔案與資料夾路徑重疊。".into()); }
            component = parent;
        }
    }
    Ok(Input { archive, sizes, _parents: parents })
}

impl Input {
    fn read(&mut self, name: &str, size: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.archive.by_name(name)?.take(size as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() != size { return Err("Canvas ZIP 項目大小與宣告不符。".into()); }
        Ok(bytes)
    }
}

fn image(bytes: &[u8]) -> Result<CanvasBundleImage> {
    let source = super::board_images::decode_image(bytes)?;
    let (w, h) = source.dimensions();
    let source = if w > 400 || h > 300 {
        image::DynamicImage::ImageRgba8(source).thumbnail(400, 300).to_rgba8()
    } else { source };
    let width = i32::try_from(source.width())?;
    let height = i32::try_from(source.height())?;
    let mut bgra = source.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        // The Win32 renderer uses a 32-bit BI_RGB DIB without alpha. Composite
        // against neutral white before discarding alpha; keep the source bytes.
        let alpha = u16::from(pixel[3]);
        for ch in &mut pixel[..3] { *ch = ((u16::from(*ch) * alpha + 255 * (255 - alpha)) / 255) as u8; }
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Ok(CanvasBundleImage { width, height, bgra })
}

impl Notebook {
    pub(crate) fn import_canvas_bundle(&self, archive: &Path) -> Result<ImportedCanvasBundle> {
        let mut input = open_input(archive)?;
        let candidates: Vec<_> = input.sizes.iter().filter(|(name, _)| canvas_name(name))
            .map(|(name, size)| (name.clone(), *size)).collect();
        let [(path, size)] = candidates.as_slice() else {
            return Err("Canvas ZIP 必須只含一份 .canvas。多畫布 Vault 請分別匯入。".into());
        };
        let doc = canvas::parse_bundle(String::from_utf8(input.read(path, *size)?)?)?;
        let names = input.sizes.keys().filter(|name| *name != path).cloned().collect();
        let report = report(&doc, path, &names);
        let keep: BTreeSet<_> = report.references.values().filter_map(|state| match state {
            CanvasAssetState::Stored(name) => Some(name.as_str()), _ => None,
        }).collect();
        let needed: i64 = keep.iter().map(|name| input.sizes[*name] as i64).sum();
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let existing: i64 = tx.query_row("SELECT coalesce(sum(length(bytes)),0) FROM canvas_bundle_assets",
            [], |r| r.get(0))?;
        if existing.saturating_add(needed) > TOTAL_ASSETS {
            return Err("已達工作區 Canvas 附件儲存上限。".into());
        }
        let title = clean_title(Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("Canvas"));
        let changed = tx.execute(
            "INSERT INTO saved_canvases(title,source)
             SELECT ?1,?2
             WHERE (SELECT count(*) FROM saved_canvases)<128
             AND (SELECT coalesce(sum(length(CAST(source AS BLOB))),0) FROM saved_canvases)
                 +length(CAST(?2 AS BLOB)) <= ?3",
            params![title, doc.as_str(), MAX_CANVAS_STORAGE_BYTES])?;
        if changed != 1 { return Err("已達工作區 Canvas 儲存上限。".into()); }
        let id = tx.last_insert_rowid();
        tx.execute("INSERT INTO canvas_bundles(canvas_id,canvas_path,inventory) VALUES(?1,?2,?3)",
            params![id, path, serde_json::to_string(&names)?])?;
        for name in keep {
            let size = input.sizes[name];
            let bytes = input.read(name, size)?;
            tx.execute("INSERT INTO canvas_bundle_assets(canvas_id,path,bytes) VALUES(?1,?2,?3)",
                params![id, name, bytes])?;
        }
        tx.commit()?;
        Ok(ImportedCanvasBundle { id, title, report })
    }

    fn canvas_bundle_source(&self, id: i64) -> Result<Option<(String, String, String)>> {
        Ok(self.connection.query_row(
            "SELECT b.canvas_path,c.source,b.inventory FROM canvas_bundles b
             JOIN saved_canvases c ON c.id=b.canvas_id WHERE b.canvas_id=?1",
            [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?)
    }

    fn canvas_bundle_paths(&self, id: i64) -> Result<BTreeSet<String>> {
        let mut query = self.connection.prepare("SELECT path FROM canvas_bundle_assets WHERE canvas_id=?1 ORDER BY path")?;
        Ok(query.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn canvas_bundle_report(&self, id: i64) -> Result<Option<CanvasBundleReport>> {
        let Some((path, raw, inventory)) = self.canvas_bundle_source(id)? else { return Ok(None) };
        let names = self.canvas_bundle_paths(id)?;
        let inventory: BTreeSet<String> = serde_json::from_str(&inventory)?;
        let mut report = report(&canvas::parse_bundle(raw)?, &path, &inventory);
        for state in report.references.values_mut() {
            if matches!(state, CanvasAssetState::Stored(name) if !names.contains(name)) {
                *state = CanvasAssetState::Missing;
            }
        }
        Ok(Some(report))
    }

    pub(crate) fn canvas_bundle_view(&self, id: i64) -> Result<Option<CanvasBundleView>> {
        let Some(report) = self.canvas_bundle_report(id)? else { return Ok(None) };
        let names = self.canvas_bundle_paths(id)?;
        let mut previews = BTreeMap::new();
        for path in names.iter()
            .filter(|p| p.rsplit_once('.').is_some_and(|(_, ext)| ["png", "jpg", "jpeg"].iter().any(|known| ext.eq_ignore_ascii_case(known))))
            .take(MAX_PREVIEWS)
        {
            let bytes: Vec<u8> = self.connection.query_row(
                "SELECT bytes FROM canvas_bundle_assets WHERE canvas_id=?1 AND path=?2",
                params![id, path], |r| r.get(0))?;
            if let Ok(image) = image(&bytes) { previews.insert(path.clone(), image); }
        }
        Ok(Some(CanvasBundleView {
            references: report.references, previews, asset_paths: names.into_iter().collect(),
        }))
    }

    pub(crate) fn save_canvas_bundle_asset(&self, id: i64, member: &str, output: &Path) -> Result<()> {
        if !portable(member) { return Err("Canvas 附件路徑無效。".into()); }
        let bytes: Vec<u8> = self.connection.query_row(
            "SELECT bytes FROM canvas_bundle_assets WHERE canvas_id=?1 AND path=?2", params![id, member],
            |r| r.get(0)).optional()?.ok_or("找不到封裝中的 Canvas 附件。")?;
        publish_new(output, |file, _| { file.write_all(&bytes)?; Ok(()) })
    }

    pub(crate) fn export_canvas_bundle(&self, id: i64, output: &Path) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        let (canvas_path, raw, _) = self.canvas_bundle_source(id)?.ok_or("這份 Canvas 不含 ZIP 封裝。")?;
        let names = self.canvas_bundle_paths(id)?;
        if !portable(&canvas_path) || !canvas_name(&canvas_path) || names.iter().any(|n| !portable(n) || n == &canvas_path) {
            return Err("Canvas ZIP 的儲存路徑無效。".into());
        }
        canvas::parse_bundle(raw.clone())?;
        publish_new(output, |file, _| {
            let mut zip = ZipWriter::new(file);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file(&canvas_path, opts)?;
            zip.write_all(raw.as_bytes())?;
            for name in names {
                let bytes: Vec<u8> = tx.query_row(
                    "SELECT bytes FROM canvas_bundle_assets WHERE canvas_id=?1 AND path=?2",
                    params![id, name], |r| r.get(0))?;
                zip.start_file(name, opts)?;
                zip.write_all(&bytes)?;
            }
            zip.finish()?;
            tx.commit()?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn exact_keys_ignore_basename_and_refuse_aliases() {
        let set = |paths: &[&str]| paths.iter().map(|s| (*s).to_owned()).collect();
        let names = set(&["甲/截圖.png", "乙/截圖.png", "Vault/研究/截圖.png"]);
        assert_eq!(resolve_reference("乙/截圖.png", "Vault/研究/圖.canvas", &names),
                   CanvasAssetState::Stored("乙/截圖.png".into()));
        let only_part = set(&["甲/截圖.png", "乙/截圖.png"]);
        assert_eq!(resolve_reference("截圖.png", "研究/圖.canvas", &only_part), CanvasAssetState::Missing);
        let ambiguous = set(&["a/圖.png", "A/圖.png"]);
        assert_eq!(resolve_reference("a/圖.png", "研究/圖.canvas", &ambiguous), CanvasAssetState::Ambiguous);
        assert_eq!(resolve_reference("../甲/截圖.png", "研究/圖.canvas", &names), CanvasAssetState::Unsafe);
    }

    #[test]
    fn source_jump_waits_for_real_selected_vault_and_resolves_exact_heading() -> Result<()> {
        use crate::archive::ImportedPage;
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let dir = super::super::tests::data_dir();
        let result = (|| -> Result<()> {
            let source = dir.join("vault.zip");
            let pack = json!({"nodes":[
                {"id":"exact","type":"file","x":0,"y":0,"width":100,"height":80,
                 "file":"甲/鏡頭.md","subpath":"#答案"},
                {"id":"basename-only","type":"file","x":120,"y":0,"width":90,"height":80,"file":"鏡頭.md"}
            ]});
            let content = "# 答案\n甲組筆記。\n";
            let mut zip = ZipWriter::new(File::create_new(&source)?);
            for (name, bytes) in [("研究/圖.canvas", pack.to_string().into_bytes()),
                ("甲/鏡頭.md", content.as_bytes().to_vec()), ("乙/鏡頭.md", b"separate sibling".to_vec())] {
                zip.start_file(name, SimpleFileOptions::default())?;
                zip.write_all(&bytes)?;
            }
            zip.finish()?;
            let bundle = db.import_canvas_bundle(&source)?.id;
            assert!(db.resolve_canvas_file_link(bundle, None, "exact")?.note_id.is_none());
            let page = |folder: &str, body: &str| ImportedPage {
                parts: vec![folder.into(), "鏡頭.md".into()], title: "鏡頭".into(), body: body.into(),
            };
            let a = db.import_vault("指定 Vault", vec![page("甲", content), page("乙", "乙組筆記")])?;
            let b = db.import_vault("其他 Vault", vec![page("甲", "其他 Vault 同路徑")])?;
            db.compare_set_canvas_vault(bundle, None, Some(a))?;
            let (scope, links) = db.canvas_file_link_snapshot(bundle)?;
            assert_eq!(scope, Some(a));
            let exact = db.resolve_canvas_file_link(bundle, scope, "exact")?;
            assert_eq!(exact.subpath.as_deref(), Some("#答案"));
            assert_eq!(db.note(exact.note_id.unwrap())?.unwrap().body, content);
            assert!(links.iter().find(|link| link.node_id == "basename-only").unwrap().note_id.is_none());
            db.compare_set_canvas_vault(bundle, Some(a), Some(b))?;
            assert!(db.resolve_canvas_file_link(bundle, Some(a), "exact").is_err());
            db.compare_set_canvas_vault(bundle, Some(b), None)?;
            assert!(db.resolve_canvas_file_link(bundle, None, "exact")?.note_id.is_none());
            assert_eq!(db.read_canvas(bundle)?.json(), &pack);
            Ok(())
        })();
        let _ = fs::remove_dir_all(dir);
        result
    }

    #[test]
    fn bundle_preserves_assets_standard_fields_and_extensions() -> Result<()> {
        let db = Notebook::open(Path::new(":memory:"), false)?;
        let p = std::env::temp_dir().join(format!("paperroom-canvas-bundle-test-{}", std::process::id()));
        fs::create_dir(&p)?;
        let result = (|| -> Result<()> {
            let source = p.join("before.zip");
            let output = p.join("after.zip");
            let attachment = p.join("saved.bin");
            let pack = json!({"nodes":[
                {"id":"g","type":"group","x":-4,"y":0,"width":40,"height":40,"background":"甲/截圖.png","backgroundStyle":"cover"},
                {"id":"n","type":"file","x":2,"y":0,"width":20,"height":10,"file":"乙/截圖.png"},
                {"id":"u","type":"vendor","payload":{"preserve":true}}],
                "edges":[{"id":"e","fromNode":"n","toNode":"g","fromEnd":"arrow","toEnd":"none","label":"反向"}]});
            let original = serde_json::to_vec(&pack)?;
            let mut z = ZipWriter::new(File::create_new(&source)?);
            for (name, bytes) in [("研究/板.canvas", original.as_slice()), ("甲/截圖.png", b"A"), ("乙/截圖.png", b"B")] {
                z.start_file(name, SimpleFileOptions::default())?;
                z.write_all(bytes)?;
            }
            z.finish()?;
            let imported = db.import_canvas_bundle(&source)?;
            assert_eq!(imported.report.unknown_nodes, 1);
            assert_eq!(db.read_canvas(imported.id)?.json(), &pack);
            db.save_canvas_bundle_asset(imported.id, "乙/截圖.png", &attachment)?;
            assert_eq!(fs::read(attachment)?, b"B");
            db.export_canvas_bundle(imported.id, &output)?;
            let again = db.import_canvas_bundle(&output)?;
            assert_eq!(db.read_canvas(again.id)?.json(), &pack);
            let mut reopened = open_input(&output)?;
            assert_eq!(reopened.read("甲/截圖.png", 1)?, b"A");
            assert_eq!(reopened.read("乙/截圖.png", 1)?, b"B");
            Ok(())
        })();
        let _ = fs::remove_dir_all(&p);
        result
    }
}
