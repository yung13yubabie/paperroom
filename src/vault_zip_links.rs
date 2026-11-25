//! Portable links address only notes verified in the archive and created in
//! the same import transaction. Identities from the archive are never looked
//! up in the destination workspace.
use super::attachments::{MAX_FILE_BYTES, MAX_STORAGE_BYTES};
use super::migration_media::{MediaPaths, MediaPlan, referenced_paths, supported};
use super::note_links::NATIVE_NOTE_PREFIX;
use super::snapshot_sha256::sha256;
use super::vault_export::{MAX_TOTAL_BYTES, PortablePaths};
use super::{MAX_NOTE_BYTES, Result, clean_title, validate_body};
use crate::archive::{ImportedPage, MAX_ENTRIES, bounded_zip_snapshot, read_bounded};
use crate::markdown::{AnchorSyntax, note_references};
use rusqlite::{Transaction, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;
use std::io::Cursor;
use std::ops::Range;
use std::path::Path;

pub(super) const MANIFEST_PATH: &str = "Paperroom-Vault-Links.json";
const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
const VERSION: u32 = 1;
const MAX_LINKS: usize = 100_000;

pub(super) struct Manifest {
    version: u32,
    notes: Vec<ManifestNote>,
    reference_count: usize,
}

struct ManifestNote {
    path: String,
    title: String,
    identity: String,
    sha256: String,
}

fn digest(body: &str) -> String {
    let mut hex = String::with_capacity(64);
    for byte in sha256(body.as_bytes()) {
        write!(hex, "{byte:02x}").expect("writing into a String");
    }
    hex
}

fn identity(id: i64, token: &str) -> String { format!("{NATIVE_NOTE_PREFIX}{id}:{token}") }

fn valid_identity(value: &str) -> bool {
    value.strip_prefix(NATIVE_NOTE_PREFIX).and_then(|s| s.split_once(':'))
        .is_some_and(|(id, token)| id.parse::<i64>().is_ok_and(|n| n > 0 && n.to_string() == id)
            && token.len() == 32 && token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

fn references(body: &str) -> Result<Vec<Range<usize>>> {
    let mut found = Vec::new();
    for reference in note_references(body) {
        let target = reference.destination.split('#').next().unwrap_or_default();
        if !target.starts_with(NATIVE_NOTE_PREFIX) { continue; }
        if !valid_identity(target) || reference.syntax != AnchorSyntax::Wiki {
            return Err("可攜 Vault ZIP 需要完整的紙間 Wiki 身分連結。請用「插入筆記連結」更新舊連結。".into());
        }
        // Confirm the parser's byte offset, then edit just the identity.
        let tail = body.get(reference.offset..).and_then(|s| s.strip_prefix("[["))
            .ok_or("無法核對紙間連結的原文位置。")?;
        let trimmed = tail.trim_start();
        if !trimmed.strip_prefix(&reference.destination)
            .is_some_and(|rest| rest.trim_start().starts_with('|') || rest.trim_start().starts_with("]]"))
        {
            return Err("有紙間連結無法和原文精確對齊，請重新插入連結。".into());
        }
        let start = reference.offset + 2 + tail.len() - trimmed.len();
        found.push(start..start + target.len());
        if found.len() > MAX_LINKS { return Err("可攜連結數量超過上限。".into()); }
    }
    Ok(found)
}

impl Manifest {
    pub(super) fn new() -> Self {
        Self { version: VERSION, notes: Vec::new(), reference_count: 0 }
    }

    pub(super) fn record(&mut self, path: &str, title: &str, id: i64, token: &str, body: &str) -> Result<()> {
        self.reference_count += references(body)?.len();
        if self.reference_count > MAX_LINKS { return Err("可攜連結數量超過上限。".into()); }
        let identity = identity(id, token);
        if !valid_identity(&identity) || title.contains('\0') || clean_title(title) != title {
            return Err("匯出的筆記身分或標題無效。".into());
        }
        self.notes.push(ManifestNote {
            path: path.to_owned(), title: title.to_owned(), identity, sha256: digest(body),
        });
        Ok(())
    }

    pub(super) fn bytes(&self) -> Result<Vec<u8>> {
        let notes = self.notes.iter().map(|note| json!({
            "path": note.path, "title": note.title, "identity": note.identity, "sha256": note.sha256,
        })).collect::<Vec<_>>();
        let data = serde_json::to_vec(&json!({"version": self.version, "notes": notes}))?;
        if data.len() > MAX_MANIFEST_BYTES { return Err("可攜 Vault 清單超過 8 MiB，請分批匯出。".into()); }
        Ok(data)
    }

    fn parse(data: &[u8]) -> Result<Self> {
        let bad = "可攜 Vault 清單格式無效。";
        let value: Value = serde_json::from_slice(data).map_err(|_| bad)?;
        let version = value["version"].as_u64().and_then(|n| u32::try_from(n).ok()).ok_or(bad)?;
        let records = value["notes"].as_array().ok_or(bad)?;
        if records.len() > MAX_ENTRIES { return Err(bad.into()); }
        let mut manifest = Self { version, notes: Vec::with_capacity(records.len()), reference_count: 0 };
        for item in records {
            let field = |key: &str| item[key].as_str().map(str::to_owned).ok_or(bad);
            manifest.notes.push(ManifestNote {
                path: field("path")?, title: field("title")?, identity: field("identity")?, sha256: field("sha256")?,
            });
        }
        // Version 1 is emitted as canonical JSON. This also refuses duplicate
        // keys and fields that the intermediate JSON value would discard.
        if manifest.bytes()? != data { return Err(bad.into()); }
        Ok(manifest)
    }
}

pub(crate) struct PortableVault {
    pub(super) pages: Vec<ImportedPage>,
    pub(super) media: MediaPlan,
    pub(super) links: LinkPlan,
}

pub(super) struct LinkPlan {
    targets: Vec<ManifestNote>,
    sources: BTreeMap<String, Vec<(Range<usize>, usize)>>,
    linked: usize,
    outside: usize,
}

impl PortableVault {
    pub(crate) fn report(&self) -> String {
        format!("{}\r\n重綁 {} 個紙間連結。另有 {} 個目的地未包含在 ZIP，保留原文。",
            self.media.report(self.pages.len(), 0), self.links.linked, self.links.outside)
    }

    pub(crate) fn counts(&self) -> (usize, usize) { (self.pages.len(), self.links.linked) }
}

pub(crate) fn read(path: &Path) -> Result<PortableVault> {
    read_snapshot(&bounded_zip_snapshot(path)?)
}

fn read_snapshot(data: &[u8]) -> Result<PortableVault> {
    let mut zip = zip::unstable::read::ZipIterable::try_new(Cursor::new(data), Default::default())?
        .enforce_header_consistency(true);
    if zip.len() > MAX_ENTRIES { return Err("可攜 Vault ZIP 超過 15,000 個成員。".into()); }
    let mut reader = Cursor::new(data);
    let mut paths = PortablePaths::default();
    let (mut text_bytes, mut asset_bytes) = (0usize, 0usize);
    let mut notes = BTreeMap::new();
    let mut assets = BTreeMap::new();
    let mut manifest: Option<Manifest> = None;
    for entry in zip.files()? {
        let entry = entry?;
        let path = entry.name()?.into_owned();
        if entry.is_dir() || entry.is_symlink() || path.contains('\\') {
            return Err("可攜 Vault ZIP 含有不支援的檔案項目。".into());
        }
        paths.add_file(&path)?;
        let note = Path::new(&path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        let limit = if path == MANIFEST_PATH { MAX_MANIFEST_BYTES }
            else if note { (MAX_TOTAL_BYTES - text_bytes).min(MAX_NOTE_BYTES) }
            else if supported(Path::new(&path)) { (MAX_STORAGE_BYTES as usize - asset_bytes).min(MAX_FILE_BYTES) }
            else { return Err("可攜 Vault ZIP 含有非預期的成員。".into()); };
        let size = entry.size();
        if size > limit as u64 { return Err("可攜 Vault ZIP 的文字或附件超過容量上限。".into()); }
        let bytes = read_bounded(entry.with_reader(&mut reader, Default::default())?, limit as u64)?;
        if bytes.len() as u64 != size { return Err("可攜 Vault ZIP 的成員大小不符。".into()); }
        if path == MANIFEST_PATH {
            manifest = Some(Manifest::parse(&bytes)?);
        } else if note {
            text_bytes += bytes.len();
            let body = String::from_utf8(bytes)?;
            validate_body(&body)?;
            notes.insert(path, body);
        } else {
            asset_bytes += bytes.len();
            assets.insert(path, bytes);
        }
    }
    let manifest = manifest.ok_or("找不到可攜 Vault 清單。一般 Vault ZIP 可解壓後用資料夾匯入。")?;
    if manifest.version != VERSION || notes.is_empty() || manifest.notes.len() != notes.len() {
        return Err("可攜 Vault 清單的版本或筆記成員數不符。".into());
    }
    let mut covered = BTreeSet::new();
    let mut destinations = HashMap::new();
    for (i, note) in manifest.notes.iter().enumerate() {
        let body = notes.get(&note.path).ok_or("可攜 Vault 清單列出的檔案不存在。")?;
        if !covered.insert(note.path.as_str()) || !valid_identity(&note.identity)
            || destinations.insert(note.identity.as_str(), i).is_some()
            || note.title.contains('\0') || clean_title(&note.title) != note.title
            || digest(body) != note.sha256
        {
            return Err("可攜 Vault 清單的筆記路徑、身分、標題或校驗不符。".into());
        }
    }

    let resolver = MediaPaths::from_paths(assets.keys().map(String::as_str));
    let mut used_assets = BTreeSet::new();
    let mut media = MediaPlan::default();
    let mut pages = Vec::with_capacity(manifest.notes.len());
    let mut sources = BTreeMap::<String, Vec<(Range<usize>, usize)>>::new();
    let (mut linked, mut outside) = (0, 0);
    for note in &manifest.notes {
        let body = notes.remove(&note.path).ok_or("筆記未經驗證。")?;
        let parts = note.path.split('/').map(str::to_owned).collect::<Vec<_>>();
        for (raw, wiki) in referenced_paths(&body) {
            if let Some(asset) = resolver.resolve(&parts, &raw, wiki) {
                if wiki && !raw.contains('/') { return Err("可攜 Vault 的附件必須有明確路徑。".into()); }
                media.add_bytes(&note.path, &asset, &assets[&asset])?;
                used_assets.insert(asset);
            }
        }
        for span in references(&body)? {
            if let Some(&target) = destinations.get(&body[span.clone()]) {
                sources.entry(note.path.clone()).or_default().push((span, target));
                linked += 1;
            } else {
                outside += 1;
            }
            if linked + outside > MAX_LINKS { return Err("可攜連結數量超過上限。".into()); }
        }
        pages.push(ImportedPage { parts, title: note.title.clone(), body });
    }
    if used_assets.len() != assets.len() { return Err("可攜 Vault ZIP 含有未能配對到筆記的來源附件。".into()); }
    media.check_notes(&pages.iter().zip(&manifest.notes).map(|(page, note)| (page, note.path.clone()))
        .collect::<Vec<_>>(), true);
    pages.sort_by(|a, b| a.parts.cmp(&b.parts));
    Ok(PortableVault { pages, media, links: LinkPlan { targets: manifest.notes, sources, linked, outside } })
}

impl LinkPlan {
    pub(super) fn rebind(self, tx: &Transaction<'_>, root: i64) -> Result<()> {
        let mut stmt = tx.prepare(
            "SELECT p.relative_path,n.id,i.token FROM vault_paths p JOIN notes n ON n.id=p.note_id
             JOIN note_link_identities i ON i.note_id=n.id WHERE p.import_root=?1 AND n.trash_root IS NULL",
        )?;
        let ids = stmt.query_map([root], |r| {
            Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))
        })?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
        for (source, edits) in self.sources {
            let id = ids.get(&source).ok_or("找不到本次匯入的來源筆記。")?.0;
            let mut body: String = tx.query_row("SELECT body FROM notes WHERE id=?1", [id], |r| r.get(0))?;
            for (span, target) in edits.into_iter().rev() {
                let target = &self.targets[target];
                let (id, token) = ids.get(&target.path).ok_or("找不到本次匯入的目標筆記。")?;
                if body.get(span.clone()) != Some(target.identity.as_str()) {
                    return Err("可攜連結的來源位置不再相符。".into());
                }
                body.replace_range(span, &identity(*id, token));
            }
            validate_body(&body)?;
            tx.execute("UPDATE notes SET body=?2 WHERE id=?1", params![id, body])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Notebook, View};
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;

    fn page(path: &str, title: &str, body: &str) -> ImportedPage {
        ImportedPage { parts: path.split('/').map(str::to_owned).collect(), title: title.into(), body: body.into() }
    }

    fn note_id(db: &Notebook, root: i64, path: &str) -> Result<i64> {
        Ok(db.connection.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            params![root, path], |row| row.get(0),
        )?)
    }

    fn scoped(db: &Notebook, id: i64) -> Result<String> {
        let token: String = db.connection.query_row(
            "SELECT token FROM note_link_identities WHERE note_id=?1", [id], |row| row.get(0),
        )?;
        Ok(identity(id, &token))
    }

    #[test]
    fn portable_round_trip_checks_paths_and_rebinds_only_original_link_bytes() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let zip = dir.join("portable.zip");
        let excluded_zip = dir.join("excluded.zip");
        let mut source = Notebook::open(&dir.join("source.sqlite3"), false)?;
        let outside = source.add(None, "同名", "工作區外")?;
        let from_path = "場/A.md";
        let to_path = "場/B.md";
        let relative = "[普通](./B.md)";
        let mut media = MediaPlan::default();
        media.add_bytes(from_path, "素材/說明.txt", b"asset")?;
        let root = source.import_vault_with_media("合成", vec![
            page(from_path, "A", "[附件](../素材/說明.txt)"),
            page(to_path, "同名", "# 鏡頭\r\n正確內容"),
        ], media)?;
        let from = note_id(&source, root, from_path)?;
        let to = note_id(&source, root, to_path)?;
        let prior = scoped(&source, to)?;
        let external = scoped(&source, outside)?;
        let original = format!("\u{feff}{relative}\r\n[[{prior}#鏡頭|保留別名]]\r\n`[[{prior}|程式碼]]`\r\n[[{external}|封存外]]\r\n[附件](../素材/說明.txt)");
        source.save(from, 1, "A", &original)?;
        source.save(to, 1, "改過的顯示名", "# 鏡頭\r\n正確內容")?;
        source.export_vault_links(root, &zip)?;
        assert_eq!(source.note(from)?.unwrap().body, original);
        let bytes = std::fs::read(&zip)?;
        let import = read(&zip)?;
        assert_eq!(import.counts(), (2, 1));
        assert_eq!(import.links.outside, 1);
        assert_eq!(import.media.count(), 1);

        let dbfile = dir.join("destination.sqlite3");
        let mut dest = Notebook::open(&dbfile, false)?;
        let decoys: Vec<_> = (0..8).map(|_| dest.add(None, "改過的顯示名", "不同內容")).collect::<Result<_>>()?;
        let imported = dest.import_portable_vault("回匯", import)?;
        let a = note_id(&dest, imported, from_path)?;
        let b = note_id(&dest, imported, to_path)?;
        let new_bound = scoped(&dest, b)?;
        assert_ne!(new_bound, prior);
        assert!(!decoys.contains(&b));
        drop(dest);
        let dest = Notebook::open(&dbfile, false)?;
        let saved = dest.note(a)?.unwrap();
        let saved_target = dest.note(b)?.unwrap();
        assert_eq!(saved_target.row.title, "改過的顯示名");
        assert_eq!(saved_target.body, "# 鏡頭\r\n正確內容");
        assert_eq!(saved.body, original.replacen(&prior, &new_bound, 1));
        assert_eq!(dest.vault_note_links(&saved)?.iter().map(|r| r.note_id).collect::<Vec<_>>(), [b, b]);
        assert_eq!(dest.related(&saved)?.0.iter().map(|r| r.id).collect::<Vec<_>>(), [b]);
        assert_eq!(dest.connection.query_row("SELECT count(*) FROM vault_push_verified WHERE note_id=?1",
            [a], |r| r.get::<_, i64>(0))?, 0);
        assert_eq!(std::fs::read(&zip)?, bytes);

        source.trash(to)?;
        source.export_vault_links(root, &excluded_zip)?;
        let import = read(&excluded_zip)?;
        assert_eq!(import.counts(), (1, 0));
        assert_eq!(import.links.outside, 2);
        assert_eq!(source.note(from)?.unwrap().body, original);
        assert!(source.note(to)?.unwrap().row.trash_root.is_some());
        drop(source);
        drop(dest);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    fn fixture_zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for &(path, bytes) in entries {
            zip.start_file(path, SimpleFileOptions::default())?;
            zip.write_all(bytes)?;
        }
        Ok(zip.finish()?.into_inner())
    }

    #[test]
    fn refuses_missing_changed_or_ambiguous_manifest_members_and_keeps_legacy_zip_import() -> Result<()> {
        let token = "0123456789abcdef0123456789abcdef";
        let body = format!("[[{}|對方]]", identity(2, token));
        let mut manifest = Manifest::new();
        manifest.record("A.md", "A", 1, token, &body)?;
        manifest.record("B.md", "B", 2, token, "正確")?;
        let bytes = manifest.bytes()?;
        let a = ("A.md", body.as_bytes());
        let b = ("B.md", "正確".as_bytes());
        let index = (MANIFEST_PATH, bytes.as_slice());
        assert!(read_snapshot(&fixture_zip(&[a,b,index])?).is_ok());
        for malformed in [vec![a,index], vec![a,("B.md",b"changed"),index],
            vec![a,b,("b.md",b"same name"),index]] {
            assert!(read_snapshot(&fixture_zip(&malformed)?).is_err());
        }
        manifest.notes[1].identity = manifest.notes[0].identity.clone();
        let duplicated = manifest.bytes()?;
        assert!(read_snapshot(&fixture_zip(&[a,b,(MANIFEST_PATH,&duplicated)])?).is_err());
        assert!(Manifest::parse(br#"{"notes":[],"version":1,"version":1}"#).is_err());
        assert!(Manifest::new().record("A.md", "A", 1, token, "[[paperroom:note:2|舊連結]]").is_err());

        let ordinary = fixture_zip(&[a,b])?;
        assert!(read_snapshot(&ordinary).is_err());
        let legacy = crate::archive::read_archive_snapshot(&ordinary, None, None)?;
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        db.import("一般 ZIP", legacy.pages)?;
        assert_eq!(db.rows(View::All, "")?.len(), 3);
        Ok(())
    }
}
