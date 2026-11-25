use crate::archive::{ImportedPage, MAX_ENTRIES};
use crate::store::{MAX_NOTE_BYTES, MAX_TITLE_CHARS, Result};
use std::collections::HashMap;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path};

const MAX_TOTAL_BYTES: usize = 80 * 1024 * 1024;
const MAX_DEPTH: usize = 64;

#[derive(Default)]
pub struct Vault {
    pub pages: Vec<ImportedPage>,
    // A skipped directory counts once; its contents are never enumerated.
    pub skipped: usize,
    pub hidden: usize,
    pub attachments: usize,
    pub(crate) media_paths: Vec<String>,
    pub links: usize,
    pub incompatible: usize,
}

#[derive(Default)]
struct Scan<'a> {
    retired: Option<&'a [String]>,
    vault: Vault,
    entries: usize,
    bytes: usize,
}

#[cfg(windows)]
fn check_volume(path: &Path) -> Result<()> {
    use std::path::Prefix;
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err("請選擇本機磁碟上的筆記庫資料夾。".into());
    };
    let letter = match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
        _ => return Err("筆記庫匯入僅支援本機磁碟。".into()),
    };
    let drive = [u16::from(letter), u16::from(b':'), u16::from(b'\\'), 0];
    // DRIVE_REMOVABLE, FIXED, CDROM and RAMDISK are local volumes.
    if !matches!(unsafe { GetDriveTypeW(drive.as_ptr()) }, 2 | 3 | 5 | 6) {
        return Err("無法確認筆記庫位於本機磁碟。".into());
    }
    Ok(())
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

pub(crate) fn hidden(name: &std::ffi::OsStr, metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
        if metadata.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0 {
            return true;
        }
    }
    #[cfg(not(windows))]
    let _ = metadata;
    name.as_encoded_bytes().starts_with(b".")
}

// Keep directory handles alive while reading their children. On Windows the
// sharing mode prevents rename/replacement and reparse-point changes.
pub(crate) fn open_entry(path: &Path, directory: bool) -> Result<File> {
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
    if is_link(&metadata) || (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err("讀取期間項目類型已改變，已取消本次匯入。".into());
    }
    Ok(file)
}

pub fn read(root: &Path) -> Result<Vault> {
    let vault = scan_for_sync(root)?;
    if vault.pages.is_empty() {
        return Err(format!("沒有可匯入的 UTF-8 Markdown 筆記；已略過 {} 個項目。", vault.skipped).into());
    }
    Ok(vault)
}

/// A preview needs to represent a valid source directory with no Markdown notes.
pub(crate) fn scan_for_sync(root: &Path) -> Result<Vault> {
    scan_folder(root, None)
}

/// Watch the old path before the normal scan can skip it. An occupied path,
/// or a parent hidden behind an unsupported entry, cannot prove a relocation.
pub(crate) fn scan_after_relocation(root: &Path, old: &str) -> Result<Vault> {
    let retired = old.split('/').map(str::to_uppercase).collect::<Vec<_>>();
    scan_folder(root, Some(&retired))
}

fn scan_folder(root: &Path, retired: Option<&[String]>) -> Result<Vault> {
    if !root.is_absolute() || root.components().any(|c| c == Component::ParentDir) {
        return Err("請選擇完整的筆記庫資料夾路徑。".into());
    }
    #[cfg(windows)]
    check_volume(root)?;
    // Check ancestors from the volume root before inspecting anything below them.
    let ancestors: Vec<_> = root.ancestors().collect();
    for path in ancestors.into_iter().rev() {
        if is_link(&fs::symlink_metadata(path)?) {
            return Err("筆記庫路徑不可經過符號連結或重新分析點。".into());
        }
    }
    if let Some(name) = root.file_name()
        && hidden(name, &fs::symlink_metadata(root)?)
    {
        return Err("請選擇筆記庫本身，避免選到隱藏設定資料夾。".into());
    }
    let mut scan = Scan { retired, ..Scan::default() };
    scan.directory(root, &mut Vec::new())?;
    let mut vault = scan.vault;
    vault.skipped = vault.hidden + vault.attachments + vault.links + vault.incompatible;
    vault.pages.sort_by(|a, b| a.parts.cmp(&b.parts));
    Ok(vault)
}

// Notebook titles are trimmed and limited. Report unrepresentable names
// instead of silently changing or merging their displayed identities.
fn fits_title(name: &str) -> bool {
    !name.is_empty() && name.trim() == name && name.chars().count() <= MAX_TITLE_CHARS
}

impl Scan<'_> {
    fn directory(&mut self, path: &Path, parts: &mut Vec<String>) -> Result<()> {
        let _directory_handle = open_entry(path, true)?;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            self.entries += 1;
            if self.entries > MAX_ENTRIES {
                return Err("筆記庫超過 15,000 個項目，請分批匯入。".into());
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            let name = entry.file_name();
            let link = is_link(&metadata);
            let private = hidden(&name, &metadata);
            if let Some(retired) = self.retired
                && let Some(leaf) = name.to_str()
                && let Some(old_part) = retired.get(parts.len())
                && leaf.to_uppercase() == *old_part
                && parts.iter().zip(retired).all(|(part, old)| part.to_uppercase() == *old)
                && (parts.len() + 1 == retired.len() || link || private || !metadata.is_dir() || !fits_title(leaf))
            {
                return Err("舊路徑仍有磁碟項目，或其上層無法驗證。".into());
            }
            if link {
                self.vault.links += 1;
            } else if private {
                self.vault.hidden += 1;
            } else if let Some(name) = name.to_str() {
                if metadata.is_dir() {
                    if !fits_title(name) {
                        self.vault.incompatible += 1;
                        continue;
                    }
                    if parts.len() + 1 >= MAX_DEPTH {
                        return Err("筆記庫資料夾層級超過上限。".into());
                    }
                    parts.push(name.to_owned());
                    self.directory(&path, parts)?;
                    parts.pop();
                } else if metadata.is_file()
                    && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                {
                    self.note(&path, name, parts)?;
                } else {
                    self.vault.attachments += 1;
                    if metadata.is_file() && crate::store::migration_media::supported(&path) {
                        let mut components = parts.clone();
                        components.push(name.to_owned());
                        self.vault.media_paths.push(components.join("/"));
                    }
                }
            } else {
                self.vault.incompatible += 1;
            }
        }
        Ok(())
    }

    fn note(&mut self, path: &Path, name: &str, parents: &[String]) -> Result<()> {
        let title = &name[..name.len() - 3];
        if !fits_title(title) {
            self.vault.incompatible += 1;
            return Ok(());
        }
        let file = open_entry(path, false)?;
        let limit = MAX_NOTE_BYTES.min(MAX_TOTAL_BYTES - self.bytes);
        if file.metadata()?.len() > limit as u64 {
            return Err("Markdown 匯入上限為單篇 4 MiB、合計 80 MiB。".into());
        }
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err("Markdown 匯入上限為單篇 4 MiB、合計 80 MiB。".into());
        }
        self.bytes += bytes.len();
        let Ok(body) = String::from_utf8(bytes) else {
            self.vault.incompatible += 1;
            return Ok(());
        };
        if body.contains('\0') {
            self.vault.incompatible += 1;
            return Ok(());
        }
        // The extension belongs to the source path. A.md and A/ are distinct.
        let mut parts = parents.to_vec();
        parts.push(name.to_owned());
        self.vault.pages.push(ImportedPage {
            parts,
            title: title.to_owned(),
            body,
        });
        Ok(())
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum MarkdownExtension {
    Md,
    Markdown,
}

fn split_extension(name: &str) -> (&str, Option<MarkdownExtension>) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("md") => (stem, Some(MarkdownExtension::Md)),
        Some((stem, ext)) if ext.eq_ignore_ascii_case("markdown") => (stem, Some(MarkdownExtension::Markdown)),
        _ => (name, None),
    }
}

type WikiNames<'a> = std::collections::HashMap<(&'a str, Option<MarkdownExtension>), Option<i64>>;

pub(crate) struct WikiIndex<'a> {
    paths: WikiNames<'a>,
    basenames: WikiNames<'a>,
}

impl<'a> WikiIndex<'a> {
    /// Index one imported vault. Paths use '/' and include the source extension.
    pub(crate) fn new(files: &'a [(i64, String)]) -> Self {
        Self::including(files, &[])
    }

    pub(crate) fn including(files: &'a [(i64, String)], archived: &'a [(i64, String)]) -> Self {
        let mut result = Self { paths: WikiNames::new(), basenames: WikiNames::new() };
        for (id, path) in files.iter().chain(archived) {
            let (stem, Some(ext)) = split_extension(path) else { continue };
            let basename = stem.rsplit('/').next().unwrap_or(stem);
            for (index, name) in [(&mut result.paths, stem), (&mut result.basenames, basename)] {
                for extension in [None, Some(ext)] {
                    index.entry((name, extension))
                        .and_modify(|found| { if *found != Some(*id) { *found = None; } })
                        .or_insert(Some(*id));
                }
            }
        }
        result
    }

    /// Resolve the destination of a parsed wiki link at note granularity.
    /// Keep local anchors, unknown targets and ambiguous basenames unresolved.
    pub(crate) fn resolve(&self, source_path: &str, requested: &str) -> Option<i64> {
        let target = requested.split('#').next()?.trim();
        if target.is_empty() || target.starts_with("//") || target.ends_with('/')
            || target.contains(['\\', ':', '\0']) || matches!(target.rsplit('/').next(), Some("." | ".."))
        {
            return None;
        }
        if !target.contains('/') {
            let key = split_extension(target);
            return self.basenames.get(&key).copied().flatten();
        }
        let relative = target.starts_with("./") || target.starts_with("../");
        let mut parts: Vec<&str> = Vec::new();
        if relative {
            if source_path.split('/').any(|part| matches!(part, "" | "." | "..")) {
                return None;
            }
            parts.extend(source_path.split('/'));
            parts.pop();
        }
        for part in target.strip_prefix('/').unwrap_or(target).split('/') {
            match part {
                "." => {}
                ".." => { parts.pop()?; }
                "" => return None,
                _ => parts.push(part),
            }
        }
        let normalized = parts.join("/");
        self.paths.get(&split_extension(&normalized)).copied().flatten()
    }
}

pub(crate) enum MarkdownTarget {
    Found(i64),
    Missing,
    Ambiguous,
}

pub(crate) struct NoteLinkPaths<'a> {
    pub(crate) wiki: WikiIndex<'a>,
    markdown: HashMap<&'a str, i64>,
}

impl<'a> NoteLinkPaths<'a> {
    pub(crate) fn new(files: &'a [(i64, String)]) -> Self {
        Self {
            wiki: WikiIndex::new(files),
            markdown: files.iter().map(|(id, path)| (path.as_str(), *id)).collect(),
        }
    }

    pub(crate) fn markdown_target(&self, source: &[String], path: &str) -> Option<i64> {
        match self.markdown_resolution(source, path) {
            MarkdownTarget::Found(id) => Some(id),
            _ => None,
        }
    }

    pub(crate) fn markdown_resolution(&self, source: &[String], path: &str) -> MarkdownTarget {
        let resolve = |resolved: String| {
            self.markdown.get(resolved.as_str()).copied().or_else(|| {
                let leaf = resolved.rsplit('/').next().unwrap_or("");
                (!leaf.contains('.')).then(|| {
                    self.markdown.get(format!("{resolved}.md").as_str()).copied()
                }).flatten()
            })
        };
        let relative = crate::store::migration_media::source_relative(source, path).and_then(resolve);
        let root = [String::new()];
        let absolute = if path.starts_with("./") || path.starts_with("../") {
            None
        } else {
            crate::store::migration_media::source_relative(&root, path).and_then(resolve)
        };
        match (relative, absolute) {
            (Some(a), Some(b)) if a != b => MarkdownTarget::Ambiguous,
            (Some(id), _) | (_, Some(id)) => MarkdownTarget::Found(id),
            _ => MarkdownTarget::Missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static SEQ: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "paperroom-vault-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn preserves_vault_text_names_and_reports_skipped_items() -> Result<()> {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let folder = "專案 1234567890abcdef1234567890abcdef";
        fs::create_dir(root.join(folder))?;
        fs::create_dir(root.join(".obsidian"))?;
        fs::write(root.join(".obsidian/private.md"), "skip")?;
        let original = "\u{feff}---\r\naliases: [舊名]\r\n---\r\n# 不同標題\r\n[[路徑/筆記]]";
        fs::write(root.join(format!("{folder}.md")), original)?;
        fs::write(root.join(folder).join("同名.MD"), "# 次頁")?;
        fs::write(root.join("同名.md"), "文字")?;
        fs::write(root.join("圖.png"), b"image")?;
        fs::write(root.join("非文字.md"), [0xff])?;
        fs::write(root.join(format!("{}.md", "x".repeat(MAX_TITLE_CHARS + 1))), "長檔名")?;
        let output = read(root)?;
        assert_eq!((output.hidden, output.attachments, output.incompatible, output.skipped), (1, 1, 2, 4));
        assert_eq!(output.pages.len(), 3);
        let top = output.pages.iter().find(|p| p.title == folder).unwrap();
        assert_eq!(top.body, original);
        assert_eq!(top.parts, [format!("{folder}.md")]);
        assert!(output.pages.iter().any(|p| p.parts == [folder, "同名.MD"]));

        let db_path = root.join("workspace.sqlite3");
        let mut notebook = crate::store::Notebook::open(&db_path, false)?;
        let imported_root = notebook.import_vault("匯入明細", output.pages)?;
        drop(notebook);
        let notebook = crate::store::Notebook::open(&db_path, false)?;
        let rows = notebook.rows(crate::store::View::All, "")?;
        let siblings: Vec<_> = rows.iter()
            .filter(|r| r.parent == Some(imported_root) && r.title == folder).collect();
        assert_eq!(siblings.len(), 2);
        let child = rows.iter().find(|r| r.title == "同名" && r.parent != Some(imported_root)).unwrap();
        let parent_id = child.parent.unwrap();
        assert!(siblings.iter().any(|r| r.id == parent_id));
        let document = siblings.iter().find(|r| r.id != parent_id).unwrap();
        assert_eq!(notebook.note(document.id)?.unwrap().body, original);
        Ok(())
    }

    #[test]
    fn refuses_an_oversize_markdown_without_partial_import() -> Result<()> {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("a.md"), "正常")?;
        File::create(fixture.0.join("oversize.md"))?.set_len(MAX_NOTE_BYTES as u64 + 1)?;
        assert!(read(&fixture.0).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn ignores_linked_files_and_directories() -> Result<()> {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        let source = outside.0.join("private.md");
        fs::write(&source, "外部筆記")?;
        fs::write(fixture.0.join("local.md"), "本機")?;
        std::os::unix::fs::symlink(&source, fixture.0.join("linked.md"))?;
        std::os::unix::fs::symlink(&outside.0, fixture.0.join("外部"))?;
        let result = read(&fixture.0)?;
        assert_eq!((result.pages.len(), result.links), (1, 2));
        assert!(read(&fixture.0.join("外部")).is_err());
        Ok(())
    }
    #[test]
    fn resolves_vault_paths_without_guessing_ambiguous_names_or_local_anchors() {
        let files = [
            (1, "入口.md".to_owned()),
            (2, "研究/甲/週報.md".to_owned()),
            (3, "研究/乙/週報.md".to_owned()),
            (4, "研究/乙/草稿.MD".to_owned()),
            (5, "研究/乙/草稿.markdown".to_owned()),
        ];
        let index = WikiIndex::new(&files);
        let source = "研究/甲/週報.md";
        for (target, id) in [
            ("入口", Some(1)), ("/入口.md", Some(1)), ("../../入口", Some(1)),
            ("研究/乙/週報#會議", Some(3)), ("../乙/週報#^summary", Some(3)),
            ("./週報", Some(2)), ("週報", None), ("研究/乙/草稿", None),
            ("草稿.MD", Some(4)), ("../乙/草稿.markdown", Some(5)),
            ("#123", None), ("#^文字", None), ("../../../入口", None),
            ("//host/入口.md", None), ("研究/乙/附件.png", None),
        ] {
            assert_eq!(index.resolve(source, target), id, "{target}");
        }
    }

}
