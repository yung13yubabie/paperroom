//! Capture only the exact local paths named by a loose .canvas file.
use super::{
    CanvasAssetState, CanvasBundleReport, MAX_ASSET, MAX_CONTENT, MAX_ENTRIES, Notebook, Result,
    canvas, canvas_name, inspect_references, portable,
};
use crate::vault::open_entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_DIRECTORY_ENTRIES: usize = 65_536;

pub(crate) struct LocalCanvasBundle {
    path: String,
    doc: canvas::Document,
    report: CanvasBundleReport,
    assets: BTreeMap<String, Vec<u8>>,
}

// Linux uses directory descriptors for each hop. Windows keeps all ancestors
// pinned using the vault reader's non-delete/non-write sharing mode.
fn directory_path(location: &Path, _handle: &File) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let _ = location;
        PathBuf::from(format!("/proc/self/fd/{}", _handle.as_raw_fd()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        location.to_owned()
    }
}

fn checked_parents(root: &Path) -> Result<Vec<File>> {
    #[cfg(windows)]
    super::super::attachments::require_local_volume(root)?;
    let mut handles = Vec::new();
    for location in root.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let candidate = match (handles.last(), location.parent(), location.file_name()) {
            (Some(parent), Some(base), Some(name)) => directory_path(base, parent).join(name),
            _ => location.to_owned(),
        };
        handles.push(open_entry(&candidate, true)?);
    }
    Ok(handles)
}

#[derive(Default)]
struct DirectoryScan {
    remaining: usize,
    children: BTreeMap<PathBuf, BTreeMap<String, Option<String>>>,
}

impl DirectoryScan {
    fn find(
        &mut self, location: &Path, handle: &File, name: &str,
    ) -> Result<Option<CanvasAssetState>> {
        if !self.children.contains_key(location) {
            let Ok(entries) = fs::read_dir(directory_path(location, handle)) else {
                return Ok(Some(CanvasAssetState::Unsafe));
            };
            let mut names = BTreeMap::new();
            for entry in entries {
                self.remaining = self.remaining.checked_sub(1)
                    .ok_or("Canvas 參照的資料夾項目過多，已取消預覽。")?;
                let Ok(entry) = entry else { return Ok(Some(CanvasAssetState::Unsafe)); };
                let Ok(name) = entry.file_name().into_string() else { continue; };
                names.entry(name.to_lowercase()).and_modify(|match_name| *match_name = None)
                    .or_insert(Some(name));
            }
            self.children.insert(location.to_owned(), names);
        }
        match self.children[location].get(&name.to_lowercase()) {
            Some(Some(exact)) if exact == name => Ok(None),
            Some(None) => Ok(Some(CanvasAssetState::Ambiguous)),
            _ => Ok(Some(CanvasAssetState::Missing)),
        }
    }

    fn read_asset(
        &mut self, root: &Path, root_handle: &File, raw: &str, remaining: usize,
    ) -> Result<std::result::Result<Vec<u8>, CanvasAssetState>> {
        if !portable(raw) || canvas_name(raw) {
            return Ok(Err(CanvasAssetState::Unsafe));
        }
        let mut location = root.to_owned();
        let mut handles = Vec::new();
        let mut components = raw.split('/').peekable();
        while let Some(name) = components.next() {
            let handle = handles.last().unwrap_or(root_handle);
            if let Some(state) = self.find(&location, handle, name)? { return Ok(Err(state)); }
            let is_directory = components.peek().is_some();
            let entry = directory_path(&location, handle).join(name);
            let Ok(opened) = open_entry(&entry, is_directory) else {
                return Ok(Err(CanvasAssetState::Unsafe));
            };
            // An exact directory entry may have been renamed since the scan.
            // Check the spelling while the already-validated handle pins it.
            #[cfg(windows)]
            if !fs::canonicalize(&entry).is_ok_and(|actual| actual.file_name() == Some(OsStr::new(name))) {
                return Ok(Err(CanvasAssetState::Missing));
            }
            if !is_directory {
                let limit = remaining.min(MAX_ASSET);
                if opened.metadata()?.len() > MAX_ASSET as u64 {
                    return Ok(Err(CanvasAssetState::Oversized));
                }
                return Ok(Ok(read_at_most(opened, limit, "Canvas 和附件合計超過 128 MiB。")?));
            }
            location.push(name);
            handles.push(opened);
        }
        Ok(Err(CanvasAssetState::Unsafe))
    }
}

fn read_at_most(file: File, limit: usize, error: &'static str) -> Result<Vec<u8>> {
    if file.metadata()?.len() > limit as u64 { return Err(error.into()); }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit { return Err(error.into()); }
    Ok(bytes)
}

impl LocalCanvasBundle {
    pub(crate) fn read(source: &Path) -> Result<Self> {
        if !source.is_absolute() || source.components().any(|p| p == Component::ParentDir) {
            return Err("請從本機資料夾選擇 .canvas 檔。".into());
        }
        let path = source.file_name().and_then(OsStr::to_str)
            .filter(|name| portable(name) && canvas_name(name)).ok_or("需要有效的 .canvas 檔名。")?;
        let root = source.parent().ok_or("找不到 Canvas 所在資料夾。")?;
        let parents = checked_parents(root)?;
        let handle = parents.last().ok_or("找不到 Canvas 所在資料夾。")?;
        let file = open_entry(&directory_path(root, handle).join(path), false)?;
        let bytes = read_at_most(file, canvas::MAX_BYTES, "JSON Canvas 超過 8 MiB。")?;
        let doc = canvas::parse_bundle(String::from_utf8(bytes)?)?;
        let mut report = inspect_references(&doc, |_| CanvasAssetState::Missing);
        if report.references.len() >= MAX_ENTRIES {
            return Err("Canvas 的附件參照超過 1,023 筆。".into());
        }
        let mut scan = DirectoryScan { remaining: MAX_DIRECTORY_ENTRIES, ..DirectoryScan::default() };
        let mut remaining = MAX_CONTENT - doc.as_str().len();
        let mut assets = BTreeMap::new();
        for (name, state) in &mut report.references {
            match scan.read_asset(root, handle, name, remaining)? {
                Ok(bytes) => {
                    remaining -= bytes.len();
                    assets.insert(name.clone(), bytes);
                    *state = CanvasAssetState::Stored(name.clone());
                }
                Err(issue) => *state = issue,
            }
        }
        Ok(Self { path: path.into(), doc, report, assets })
    }

    pub(crate) fn description(&self) -> String {
        format!(
            "附件只從所選 .canvas 所在資料夾及其子資料夾讀取，依原路徑和大小寫配對。\r\n\
            不會讀取上層、符號連結、其他 .canvas 或遠端網址。\r\n\
            確認後建立工作區副本；缺少或無法讀取的附件保留原參照。\r\n\r\n{}",
            self.report.describe(true),
        )
    }

    pub(crate) fn save(mut self, db: &Notebook) -> Result<(i64, String)> {
        let names: BTreeSet<_> = self.assets.keys().cloned().collect();
        let keep = names.iter().map(|name| (name.as_str(), self.assets[name].len())).collect();
        db.store_canvas_bundle(&self.path, &self.doc, &names, keep, |name, _| {
            self.assets.remove(name).ok_or_else(|| "找不到已預覽的 Canvas 附件。".into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn local_preview_is_read_only_and_the_same_files_survive_reopen_and_zip() -> Result<()> {
        let root = super::super::super::tests::data_dir();
        let result = (|| -> Result<()> {
            let scope = root.join("source");
            let media = scope.join("附件");
            fs::create_dir_all(&media)?;
            let mut outside = File::create_new(root.join("outside.png"))?;
            outside.write_all(b"not in scope")?;
            fs::write(media.join("封面.png"), b"cover preview bytes")?;
            fs::write(media.join("圖卡.jpg"), b"card preview bytes")?;
            let canvas = scope.join("研究.canvas");
            let raw = json!({
                "nodes": [
                    {"id":"g","type":"group","x":-50,"y":-25,"width":360,"height":240,
                     "label":"群組","background":"附件/封面.png","backgroundStyle":"repeat"},
                    {"id":"f","type":"file","x":0,"y":5,"width":80,"height":90,"file":"附件/圖卡.jpg"},
                    {"id":"m","type":"file","x":120,"y":10,"width":80,"height":90,"file":"附件/缺少.png"},
                    {"id":"u","type":"file","x":0,"y":5,"width":80,"height":90,"file":"../outside.png"},
                    {"id":"c","type":"file","x":0,"y":5,"width":80,"height":90,"file":"研究.canvas"},
                    {"id":"l","type":"link","x":380,"y":0,"width":100,"height":60,"url":"https://example.invalid"}
                ],
                "edges":[{"id":"e","fromNode":"f","fromSide":"right","fromEnd":"arrow",
                           "toNode":"l","toSide":"left","toEnd":"none","label":"回望","extra":7}]
            }).to_string();
            fs::write(&canvas, &raw)?;
            let db_path = root.join("test.sqlite3");
            let db = Notebook::open(&db_path, false)?;
            let report = LocalCanvasBundle::read(&canvas)?;
            assert!(report.description().contains("將保存：附件/封面.png"));
            assert_eq!(report.report.references["附件/缺少.png"], CanvasAssetState::Missing);
            assert_eq!(report.report.references["../outside.png"], CanvasAssetState::Unsafe);
            assert_eq!(report.report.references["研究.canvas"], CanvasAssetState::Unsafe);
            assert_eq!(report.report.references.len(), 5);
            drop(report);
            assert!(db.saved_canvases()?.is_empty());

            let report = LocalCanvasBundle::read(&canvas)?;
            fs::write(media.join("封面.png"), b"changed after preview")?;
            let (id, _) = report.save(&db)?;
            drop(db);
            let reopened = Notebook::open(&db_path, false)?;
            assert_eq!(reopened.read_canvas(id)?.as_str(), raw);
            assert_eq!(reopened.canvas_bundle_view(id)?.unwrap().asset_paths.len(), 2);
            let zip = root.join("export.zip");
            reopened.export_canvas_bundle(id, &zip)?;
            let clean = Notebook::open(Path::new(":memory:"), false)?;
            let imported = clean.import_canvas_bundle(&zip)?;
            assert_eq!(clean.read_canvas(imported.id)?.as_str(), raw);
            let extracted = root.join("saved.png");
            clean.save_canvas_bundle_asset(imported.id, "附件/封面.png", &extracted)?;
            assert_eq!(fs::read(extracted)?, b"cover preview bytes");
            assert_eq!(fs::read(&canvas)?, raw.as_bytes());
            Ok(())
        })();
        let _ = fs::remove_dir_all(root);
        result
    }

    #[test]
    fn refuses_wrong_case_oversized_files_and_out_of_scope_paths() -> Result<()> {
        let root = super::super::super::tests::data_dir();
        let result = (|| -> Result<()> {
            let scope = root.join("scope");
            fs::create_dir(&scope)?;
            fs::write(scope.join("Picture.png"), b"exact attachment")?;
            fs::write(root.join("external.png"), b"out of scope")?;
            File::create_new(scope.join("oversized.png"))?.set_len(MAX_ASSET as u64 + 1)?;
            #[cfg(target_os = "linux")]
            {
                fs::write(scope.join("picture.png"), b"different content")?;
                std::os::unix::fs::symlink(&root, scope.join("alias"))?;
            }
            let source = scope.join("case.canvas");
            let refs = [
                "picture.png", "alias/external.png", "../external.png",
                "https://example.invalid/remote.png", "oversized.png",
            ];
            let nodes: Vec<_> = refs.iter().enumerate().map(|(i, path)| {
                json!({"id":i.to_string(),"type":"file","x":0,"y":0,"width":100,"height":80,"file":path})
            }).collect();
            fs::write(&source, json!({"nodes":nodes,"edges":[]}).to_string())?;
            let preview = LocalCanvasBundle::read(&source)?;
            #[cfg(target_os = "linux")]
            assert_eq!(preview.report.references["picture.png"], CanvasAssetState::Ambiguous);
            #[cfg(not(target_os = "linux"))]
            assert_eq!(preview.report.references["picture.png"], CanvasAssetState::Missing);
            #[cfg(target_os = "linux")]
            assert_eq!(preview.report.references["alias/external.png"], CanvasAssetState::Unsafe);
            assert_eq!(preview.report.references["../external.png"], CanvasAssetState::Unsafe);
            assert_eq!(preview.report.references["https://example.invalid/remote.png"], CanvasAssetState::Unsafe);
            assert_eq!(preview.report.references["oversized.png"], CanvasAssetState::Oversized);
            assert!(preview.assets.is_empty());
            Ok(())
        })();
        let _ = fs::remove_dir_all(root);
        result
    }
}
