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
    vault_path: Option<String>,
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
    fn new() -> Self {
        Self { remaining: MAX_DIRECTORY_ENTRIES, ..Self::default() }
    }

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

    // The caller validates the relative path before walking its components.
    fn open_exact(
        &mut self, root: &Path, root_handle: &File, raw: &str,
    ) -> Result<std::result::Result<(File, Vec<File>), CanvasAssetState>> {
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
                return Ok(Ok((opened, handles)));
            }
            location.push(name);
            handles.push(opened);
        }
        Ok(Err(CanvasAssetState::Unsafe))
    }

    fn read_asset(
        &mut self, root: &Path, root_handle: &File, raw: &str, remaining: usize,
    ) -> Result<std::result::Result<Vec<u8>, CanvasAssetState>> {
        if !portable(raw) || canvas_name(raw) {
            return Ok(Err(CanvasAssetState::Unsafe));
        }
        let (opened, _parents) = match self.open_exact(root, root_handle, raw)? {
            Ok(file) => file,
            Err(issue) => return Ok(Err(issue)),
        };
        if opened.metadata()?.len() > MAX_ASSET as u64 {
            return Ok(Err(CanvasAssetState::Oversized));
        }
        let limit = remaining.min(MAX_ASSET);
        Ok(Ok(read_at_most(opened, limit, "Canvas 和附件合計超過 128 MiB。")?))
    }
}

fn read_at_most(file: File, limit: usize, error: &'static str) -> Result<Vec<u8>> {
    if file.metadata()?.len() > limit as u64 { return Err(error.into()); }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit { return Err(error.into()); }
    Ok(bytes)
}

fn absolute_without_parent(path: &Path) -> bool {
    path.is_absolute() && !path.components().any(|part| part == Component::ParentDir)
}

pub(crate) fn read_base_card_file(root:&Path, relative:&str) -> Result<Vec<u8>> {
    if !absolute_without_parent(root) || !portable(relative) || !relative.rsplit_once('.').is_some_and(|(_,e)| e.eq_ignore_ascii_case("base")) {
        return Err("需要本機 Vault 及完整的 Bases 相對路徑。".into());
    }
    let dirs=checked_parents(root)?;
    let handle=dirs.last().ok_or("無法開啟 Vault。")?;
    let (file,_parents)=DirectoryScan::new().open_exact(root,handle,relative)?
        .map_err(|_|"無法在指定 Vault 找到唯一且安全的同路徑 Bases 檔案。")?;
    read_at_most(file,super::super::obsidian_base::MAX_BYTES,"Bases 原檔超過上限。")
}

fn root_canvas_name(path: &str, assets: &BTreeMap<String, Vec<u8>>) -> Result<String> {
    let occupied: BTreeSet<_> = assets.keys().filter_map(|name| name.split('/').next())
        .map(str::to_lowercase).collect();
    if !occupied.contains(&path.to_lowercase()) { return Ok(path.into()); }
    // A directory at the vault root can itself have a .canvas suffix.
    (1..=MAX_ENTRIES).map(|i| format!("Canvas-{i}.canvas"))
        .find(|name| !occupied.contains(&name.to_lowercase()))
        .ok_or_else(|| "找不到可使用的 Canvas ZIP 畫布檔名。".into())
}

impl LocalCanvasBundle {
    pub(crate) fn read(source: &Path) -> Result<Self> {
        if !absolute_without_parent(source) {
            return Err("請從本機資料夾選擇 .canvas 檔。".into());
        }
        let path = source.file_name().and_then(OsStr::to_str)
            .filter(|name| portable(name) && canvas_name(name)).ok_or("需要有效的 .canvas 檔名。")?;
        let root = source.parent().ok_or("找不到 Canvas 所在資料夾。")?;
        let parents = checked_parents(root)?;
        let handle = parents.last().ok_or("找不到 Canvas 所在資料夾。")?;
        let file = open_entry(&directory_path(root, handle).join(path), false)?;
        Self::capture(file, root, handle, path, None, DirectoryScan::new())
    }

    pub(crate) fn read_vault(source: &Path, vault: &Path) -> Result<Self> {
        if !absolute_without_parent(source) || !absolute_without_parent(vault) {
            return Err("請選擇本機的 Vault 資料夾及其中的 .canvas 檔。".into());
        }
        let relative = source.strip_prefix(vault)
            .map_err(|_| "請選擇位於已選 Vault 內的 .canvas 檔。")?;
        let relative = relative.components().map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        }).collect::<Option<Vec<_>>>().ok_or("Vault 內的 .canvas 路徑無效。")?.join("/");
        if !portable(&relative) || !canvas_name(&relative) {
            return Err("Vault 內的 .canvas 路徑無效。".into());
        }
        let path = relative.rsplit('/').next().unwrap_or(&relative);
        let parents = checked_parents(vault)?;
        let handle = parents.last().ok_or("找不到已選 Vault。")?;
        let mut scan = DirectoryScan::new();
        let (file, _source_parents) = scan.open_exact(vault, handle, &relative)?
            .map_err(|issue| match issue {
                CanvasAssetState::Ambiguous => "Vault 內的畫布路徑有大小寫歧義。",
                CanvasAssetState::Missing => "在已選 Vault 中找不到這份畫布的精確路徑。",
                _ => "無法安全讀取已選 Vault 內的畫布。",
            })?;
        Self::capture(file, vault, handle, path, Some(&relative), scan)
    }

    fn capture(
        file: File, root: &Path, handle: &File, path: &str, vault_path: Option<&str>,
        mut scan: DirectoryScan,
    ) -> Result<Self> {
        let bytes = read_at_most(file, canvas::MAX_BYTES, "JSON Canvas 超過 8 MiB。")?;
        let doc = canvas::parse_bundle(String::from_utf8(bytes)?)?;
        let mut report = inspect_references(&doc, |_| CanvasAssetState::Missing);
        if report.references.len() >= MAX_ENTRIES {
            return Err("Canvas 的附件參照超過 1,023 筆。".into());
        }
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
        // Put vault-mode canvases at the ZIP root so even existing readers
        // only try the recorded root-relative paths, with no parent fallbacks.
        let path = if vault_path.is_some() { root_canvas_name(path, &assets)? } else { path.into() };
        Ok(Self { path, doc, report, assets, vault_path: vault_path.map(str::to_owned) })
    }

    pub(crate) fn description(&self) -> String {
        if let Some(relative) = &self.vault_path {
            return format!(
                "Vault 中的畫布：{relative}\r\n\
                附件只依所選 Vault 根目錄下的原路徑和大小寫配對。\r\n\
                匯出 ZIP 會把「{}」放在最外層，附件保留原相對路徑。\r\n\
                確認後建立工作區副本。缺件和無法讀取的路徑保留原參照。\r\n\r\n{}",
                self.path, self.report.describe(true),
            );
        }
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

    #[test]
    fn vault_and_canvas_folder_choose_distinct_pngs_and_keep_them_through_zip_reopen() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let result = (|| -> Result<()> {
            let vault = dir.join("Vault");
            let boards = vault.join("Boards");
            for (root, frame, background) in [
                (&vault, [21, 116, 183], [23, 145, 106]),
                (&boards, [176, 67, 130], [188, 101, 36]),
            ] {
                fs::create_dir_all(root.join("Media"))?;
                image::RgbImage::from_pixel(12, 12, image::Rgb(frame)).save(root.join("Media/Frame.png"))?;
                image::RgbImage::from_pixel(12, 12, image::Rgb(background)).save(root.join("Media/Background.png"))?;
            }
            fs::write(boards.join("Media/OnlyInBoards.txt"), "Decoy for vault mode")?;
            let source = boards.join("Scene.canvas");
            let raw = json!({"nodes":[
                {"id":"g","type":"group","x":-200,"y":-150,"width":400,"height":300,
                 "label":"場景","background":"Media/Background.png","backgroundStyle":"cover"},
                {"id":"f","type":"file","x":-100,"y":-60,"width":200,"height":120,"file":"Media/Frame.png"},
                {"id":"m","type":"file","x":220,"y":0,"width":110,"height":90,"file":"Media/OnlyInBoards.txt"},
                {"id":"a","type":"file","x":220,"y":100,"width":110,"height":90,"file":"MEDIA/Frame.png"},
                {"id":"l","type":"link","x":220,"y":205,"width":110,"height":90,"url":"https://example.invalid"}
            ],"edges":[{"id":"e","fromNode":"f","toNode":"m","toEnd":"arrow"}]}).to_string();
            fs::write(&source, &raw)?;

            let workspace = dir.join("workspace.sqlite3");
            let db = Notebook::open(&workspace, false)?;
            let root_preview = LocalCanvasBundle::read_vault(&source, &vault)?;
            assert!(root_preview.description().contains("Boards/Scene.canvas"));
            assert_eq!(root_preview.report.references["Media/OnlyInBoards.txt"], CanvasAssetState::Missing);
            assert_eq!(root_preview.report.references["MEDIA/Frame.png"], CanvasAssetState::Missing);
            assert_eq!(root_preview.assets.len(), 2);
            let local_preview = LocalCanvasBundle::read(&source)?;
            assert_eq!(local_preview.assets.len(), 3);
            let root_id = root_preview.save(&db)?.0;
            let local_id = local_preview.save(&db)?.0;
            drop(db);
            let db = Notebook::open(&workspace, false)?;
            for (id, root, label) in [(root_id, &vault, "vault"), (local_id, &boards, "folder")] {
                assert_eq!(db.read_canvas(id)?.as_str(), raw);
                let output = dir.join(format!("{label}.zip"));
                db.export_canvas_bundle(id, &output)?;
                let archive = zip::ZipArchive::new(File::open(&output)?)?;
                let members = archive.file_names().collect::<std::result::Result<Vec<_>, _>>()?;
                assert!(members.iter().any(|member| member == "Scene.canvas"));
                assert!(!members.iter().any(|member| member.starts_with("Boards/")));
                let isolated = dir.join(format!("{label}.sqlite3"));
                let new_db = Notebook::open(&isolated, false)?;
                let copy = new_db.import_canvas_bundle(&output)?.id;
                drop(new_db);
                let new_db = Notebook::open(&isolated, false)?;
                assert_eq!(new_db.read_canvas(copy)?.as_str(), raw);
                let report = new_db.canvas_bundle_report(copy)?.unwrap();
                if id == root_id {
                    assert_eq!(report.references["Media/OnlyInBoards.txt"], CanvasAssetState::Missing);
                }
                for name in ["Frame.png", "Background.png"] {
                    let member = format!("Media/{name}");
                    let written = dir.join(format!("{label}-{name}"));
                    new_db.save_canvas_bundle_asset(copy, &member, &written)?;
                    assert_eq!(fs::read(written)?, fs::read(root.join(member))?);
                }
            }
            assert_eq!(fs::read(source)?, raw.as_bytes());
            Ok(())
        })();
        let _ = fs::remove_dir_all(dir);
        result
    }

    #[test]
    fn vault_rejects_an_outside_canvas_and_avoids_a_zip_name_collision() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let result = (|| -> Result<()> {
            let vault = dir.join("Vault");
            let boards = vault.join("Boards");
            fs::create_dir_all(&boards)?;
            fs::create_dir(vault.join("Scene.canvas"))?;
            fs::write(vault.join("Scene.canvas/Frame.txt"), "Referenced file")?;
            let source = boards.join("Scene.canvas");
            let raw = json!({"nodes":[
                {"id":"f","type":"file","x":0,"y":0,"width":100,"height":90,"file":"Scene.canvas/Frame.txt"},
                {"id":"a","type":"file","x":120,"y":0,"width":100,"height":90,"file":"Frames/Tile.txt"}
            ]}).to_string();
            fs::write(&source, &raw)?;
            let outside = dir.join("Scene.canvas");
            fs::write(&outside, &raw)?;
            assert!(LocalCanvasBundle::read_vault(&outside, &vault).err().unwrap().to_string().contains("位於已選 Vault 內"));

            fs::create_dir(vault.join("Frames"))?;
            fs::write(vault.join("Frames/Tile.txt"), "One match")?;
            #[cfg(target_os = "linux")]
            {
                fs::create_dir(vault.join("frames"))?;
                fs::write(vault.join("frames/Tile.txt"), "Another spelling")?;
            }
            let bundle = LocalCanvasBundle::read_vault(&source, &vault)?;
            assert_eq!(bundle.path, "Canvas-1.canvas");
            #[cfg(target_os = "linux")]
            {
                assert!(bundle.description().contains("歧義：Frames/Tile.txt"));
                assert!(!bundle.assets.contains_key("Frames/Tile.txt"));
            }
            let db = Notebook::open(Path::new(":memory:"), false)?;
            let id = bundle.save(&db)?.0;
            let zip = dir.join("valid.zip");
            db.export_canvas_bundle(id, &zip)?;
            let imported = db.import_canvas_bundle(&zip)?;
            assert_eq!(db.read_canvas(imported.id)?.as_str(), raw);
            Ok(())
        })();
        let _ = fs::remove_dir_all(dir);
        result
    }
}
