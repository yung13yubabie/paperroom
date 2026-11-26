//! Reconcile an externally moved Markdown source without editing the Vault disk.
use super::super::{
    Notebook, Result,
    canvas_links::{CanvasRelocation, plan_vault_relocation},
};
use crate::{markdown::block_anchors::{BlockLookup, resolve_blocks}, vault::NoteLinkPaths};
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{collections::{BTreeMap, HashMap}, ffi::OsStr, ops::Range, path::{Path, PathBuf}};

#[derive(PartialEq, Eq)]
struct Edit {
    id: i64,
    path: String,
    body: String,
    links: usize,
    blocks: Vec<BlockDestination>,
}

#[derive(PartialEq, Eq)]
pub(crate) struct Relocation {
    target: i64,
    root: i64,
    directory: PathBuf,
    old: String,
    next: String,
    revisions: Vec<(i64, String, i64)>,
    edits: Vec<Edit>,
    canvases: Vec<CanvasRelocation>,
}

impl Relocation {
    pub(crate) fn report(&self) -> String {
        let count: usize = self.edits.iter().map(|edit| edit.links).sum();
        let mut text = format!(
            "來源資料夾：{}\r\n原路徑：{}\r\n新路徑：{}\r\n保留筆記 ID #{}、標題與工作區位置。\r\n從磁碟採用 {} 篇筆記中已核對的 {} 個連結目的地變更。磁碟檔案維持原狀。",
            self.directory.display(), self.old, self.next, self.target, self.edits.len(), count
        );
        for edit in self.edits.iter().take(40) {
            text.push_str(&format!("\r\n{}　{} 個連結", edit.path, edit.links));
        }
        if self.edits.len() > 40 {
            text.push_str(&format!("\r\n另有 {} 篇。", self.edits.len() - 40));
        }
        if let Some(note) = self.edits.iter().find(|edit| edit.id == self.target) {
            text.push_str(&format!(
                "\r\n已核對搬移筆記的 {} 個相對傳出連結。",
                note.links
            ));
        }
        let note_blocks: usize = self.edits.iter().map(|edit| edit.blocks.len()).sum();
        let canvas_blocks: usize = self.canvases.iter().map(|canvas| canvas.block_references.len()).sum();
        if note_blocks != 0 || canvas_blocks != 0 {
            text.push_str(&format!(
                "\r\n已核對 {note_blocks} 個筆記區塊連結及 {canvas_blocks} 個 Canvas 區塊參照。"
            ));
        }
        let canvas_nodes: usize = self.canvases.iter().map(|canvas| canvas.nodes.len()).sum();
        text.push_str(&format!(
            "\r\n更新 {} 張 Canvas 的 {canvas_nodes} 個檔案節點，保留既有節點 ID 及子路徑。",
            self.canvases.len()
        ));
        for canvas in self.canvases.iter().take(20) {
            text.push_str(&format!("\r\n{} (#{})　{} 個節點", canvas.title, canvas.id, canvas.nodes.len()));
            for id in canvas.nodes.iter().take(10) {
                let abbreviated: String = id.chars().filter(|c| !c.is_control()).take(80).collect();
                text.push_str(&format!("\r\n  {abbreviated}"));
            }
            if canvas.nodes.len() > 10 {
                text.push_str(&format!("\r\n  另有 {} 個節點。", canvas.nodes.len() - 10));
            }
        }
        if self.canvases.len() > 20 {
            text.push_str(&format!("\r\n另有 {} 張 Canvas。", self.canvases.len() - 20));
        }
        text
    }
}

pub(super) fn safe_note_path(path: &str) -> Result<()> {
    let mut components = path.split('/');
    if !path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md"))
        || components.clone().count() > 64
        || !components.all(|piece| {
            !piece.starts_with('.') && piece.trim() == piece && piece.encode_utf16().count() <= 255
                && crate::output::windows_output_name(OsStr::new(piece)).is_ok()
        })
    {
        return Err("請使用合法的 Vault 相對 .md 路徑，例如「訪談/新鏡頭.md」。".into());
    }
    Ok(())
}

// Reject an exact collision, a file/directory collision, or differently cased
// spellings of the same path component. Trash paths still reserve their names.
pub(super) fn paths_conflict(left: &str, right: &str) -> bool {
    for (a, b) in left.split('/').zip(right.split('/')) {
        if a != b { return a.to_uppercase() == b.to_uppercase(); }
    }
    true
}

pub(super) fn origin(conn: &Connection, note: i64) -> Result<(i64, String)> {
    conn.query_row(
        "SELECT p.import_root,p.relative_path FROM vault_paths p JOIN notes n ON n.id=p.note_id
         JOIN notes r ON r.id=p.import_root WHERE n.id=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL",
        [note], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?.ok_or("請先開啟這個 Vault 已匯入且仍使用中的原筆記。".into())
}

struct Reference {
    destination: String,
    target: Option<i64>,
    path_span: Option<Range<usize>>,
    unresolved_relative: bool,
    explicit_relative: bool,
}

fn raw_path_span(raw: &str, range: Range<usize>, link: LinkType, dest: &str) -> Option<Range<usize>> {
    let (offset, untrimmed) = match link {
        LinkType::WikiLink { .. } => {
            let start = if raw.starts_with("![[") { 3 } else if raw.starts_with("[[") { 2 } else { return None; };
            let inner = raw.get(start..)?.strip_suffix("]]")?;
            (start, inner.split(['#', '|']).next()?)
        }
        LinkType::Inline => {
            let (label, rest) = raw.rsplit_once("](")?;
            let inner = rest.strip_suffix(')')?;
            let (extra, inner) = inner.strip_prefix('<').and_then(|s| s.strip_suffix('>'))
                .map_or((0, inner), |wrapped| (1, wrapped));
            if extra == 0 && inner.chars().any(char::is_whitespace) { return None; }
            (label.len() + 2 + extra, inner.split('#').next()?)
        }
        _ => return None,
    };
    if untrimmed.is_empty() || untrimmed != dest.split('#').next()? { return None; }
    let start = range.start + offset;
    Some(start..start + untrimmed.len())
}

fn references(body: &str, source: &str, id: i64, links: &NoteLinkPaths<'_>) -> Vec<Reference> {
    let parts = source.split('/').map(str::to_owned).collect::<Vec<_>>();
    Parser::new_ext(body, Options::ENABLE_WIKILINKS).into_offset_iter().filter_map(|(event, range)| {
        let (kind, dest) = match event {
            Event::Start(Tag::Link { link_type, dest_url, .. } | Tag::Image { link_type, dest_url, .. }) =>
                (link_type, dest_url),
            _ => return None,
        };
        let wiki = matches!(kind, LinkType::WikiLink { .. });
        let raw = body.get(range.clone())?;
        let path = dest.split('#').next().unwrap_or("");
        let target = if dest.starts_with('#') {
            Some(id)
        } else if wiki {
            links.wiki.resolve(source, &dest)
        } else {
            links.markdown_target(&parts, path)
        };
        let explicit_relative = path.starts_with("./") || path.starts_with("../");
        let unresolved_relative = target.is_none() && !path.is_empty()
            && !path.starts_with('/') && !path.contains(':')
            && (!wiki || explicit_relative);
        let path_span = raw_path_span(raw, range, kind, &dest);
        Some(Reference { destination: dest.into_string(), target, path_span, unresolved_relative, explicit_relative })
    }).collect()
}

#[derive(PartialEq, Eq)]
struct BlockDestination {
    target: i64,
    id: String,
}

struct LinkChanges {
    count: usize,
    blocks: Vec<BlockDestination>,
}

fn link_changes(
    before: &str, old_source: &str, prior: &NoteLinkPaths<'_>, after: &str, new_source: &str, next: &NoteLinkPaths<'_>,
    source_id: i64, moving: i64,
) -> Option<LinkChanges> {
    let earlier = references(before, old_source, source_id, prior);
    let later = references(after, new_source, source_id, next);
    if earlier.len() != later.len() { return None; }
    let moved_parent = old_source.rsplit_once('/').map(|(parent, _)| parent)
        != new_source.rsplit_once('/').map(|(parent, _)| parent);
    let (mut left, mut right) = (0, 0);
    let mut changes = LinkChanges { count: 0, blocks: Vec::new() };
    for (old, new) in earlier.into_iter().zip(later) {
        if old.target != new.target || (moved_parent && old.unresolved_relative) { return None; }
        if old.destination == new.destination { continue; }
        let id = old.target?;
        if source_id == moving {
            if !moved_parent || !old.explicit_relative || !new.explicit_relative { return None; }
        } else if id != moving {
            return None;
        }
        let (old_path, new_path) = (old.path_span?, new.path_span?);
        if before.get(left..old_path.start)? != after.get(right..new_path.start)? { return None; }
        (left, right) = (old_path.end, new_path.end);
        changes.count += 1;
        if let Some(block) = old.destination.split_once('#').and_then(|(_, anchor)| anchor.strip_prefix('^')) {
            changes.blocks.push(BlockDestination { target: id, id: block.to_owned() });
        }
    }
    (before.get(left..)? == after.get(right..)?).then_some(changes)
}

enum BlockReferenceSite<'a> {
    Note(&'a str),
    Canvas { title: &'a str, node: &'a str },
}

fn verify_block_destinations(
    target: i64, paths: &[(i64, String)], disk: &HashMap<String, String>, notes: &[Edit], canvases: &[CanvasRelocation],
) -> Result<()> {
    let mut references = BTreeMap::<i64, Vec<(&str, BlockReferenceSite<'_>)>>::new();
    for note in notes {
        for block in &note.blocks {
            references.entry(block.target).or_default().push((&block.id, BlockReferenceSite::Note(&note.path)));
        }
    }
    for canvas in canvases {
        for (node, id) in &canvas.block_references {
            references.entry(target).or_default().push((id, BlockReferenceSite::Canvas { title: &canvas.title, node }));
        }
    }
    let paths_by_id = paths.iter().map(|(id, path)| (*id, path.as_str())).collect::<HashMap<_, _>>();
    for (id, sites) in references {
        let path = paths_by_id.get(&id).ok_or("無法核對區塊所屬的目標筆記。")?;
        let body = disk.get(*path).ok_or("無法核對區塊所在的磁碟檔案。")?;
        let found = resolve_blocks(body, sites.iter().map(|(id, _)| *id));
        for (block, site) in sites {
            let reason = match found.get(block) {
                Some(BlockLookup::Found { .. }) => continue,
                Some(BlockLookup::Ambiguous) => "在目標筆記中有重複的區塊 ID",
                Some(BlockLookup::Unsupported) => "使用了無效的區塊 ID",
                _ => "在目標筆記中找不到這個區塊 ID",
            };
            let location = match site {
                BlockReferenceSite::Note(path) => format!("{path} 的區塊連結"),
                BlockReferenceSite::Canvas { title, node } => format!("Canvas「{title}」的節點「{node}」參照"),
            };
            return Err(format!("{location} #^{block} {reason}。").into());
        }
    }
    Ok(())
}

fn has_identity_text(body: &str) -> bool {
    use pulldown_cmark::TagEnd;
    let mut skip = 0usize;
    let (_, prose) = crate::store::vault_search::frontmatter(body);
    for event in Parser::new_ext(prose, Options::ENABLE_WIKILINKS) {
        match event {
            Event::Start(Tag::Link { .. } | Tag::Image { .. } | Tag::CodeBlock(_)) => skip += 1,
            Event::End(TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock) => skip = skip.saturating_sub(1),
            Event::Text(text) if skip == 0 && text.chars().any(char::is_alphanumeric) => return true,
            _ => {}
        }
    }
    false
}

fn preflight(conn: &Connection, target: i64, directory: &Path, next: &str) -> Result<Relocation> {
    let (root, old) = origin(conn, target)?;
    safe_note_path(&old)?;
    safe_note_path(next)?;
    if old.to_uppercase() == next.to_uppercase() {
        return Err("請選擇新路徑；僅改大小寫的變更無法安全判定。".into());
    }
    let disk = crate::vault::scan_after_relocation(directory, &old)?;
    let disk = disk.pages.into_iter().map(|page| (page.parts.join("/"), page.body)).collect::<HashMap<_, _>>();
    let candidate = disk.get(next).ok_or("磁碟找不到完全相符的新路徑；請核對檔名和大小寫。")?;
    if candidate.trim().is_empty() {
        return Err("空白筆記無法用位元相符驗證來源。".into());
    }

    let mut query = conn.prepare(
        "SELECT p.note_id,p.relative_path,n.version,n.trash_root IS NULL
         FROM vault_paths p JOIN notes n ON n.id=p.note_id WHERE p.import_root=?1 ORDER BY p.relative_path"
    )?;
    let records = query.query_map([root], |r| Ok((
        r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?,
    )))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if records.iter().any(|(id, path, _, _)| *id != target && paths_conflict(next, path)) {
        return Err("新路徑與已匯入檔案或資料夾衝突（含大小寫及垃圾桶）。".into());
    }
    let mapped = records.iter().map(|(_, path, _, _)| path.as_str()).collect::<std::collections::HashSet<_>>();
    if disk.iter().any(|(path, body)| path != next && body == candidate && !mapped.contains(path.as_str())) {
        return Err("有多個未配對的磁碟筆記與新檔內容一致，無法驗證唯一來源。".into());
    }

    let before = records.iter().filter(|record| record.3).map(|r| (r.0, r.1.clone())).collect::<Vec<_>>();
    let after = before.iter().map(|(id, path)| (*id, if *id == target { next.to_owned() } else { path.clone() }))
        .collect::<Vec<_>>();
    let prior = NoteLinkPaths::new(&before);
    let following = NoteLinkPaths::new(&after);
    let mut revisions = Vec::new();
    let mut edits = Vec::new();
    let mut bodies = conn.prepare(
        "SELECT n.body FROM notes n JOIN vault_sync_base b ON b.note_id=n.id
         JOIN vault_push_verified v ON v.note_id=n.id WHERE n.id=?1 AND n.body=b.body",
    )?;
    for (id, path, version, active) in &records {
        if !active { continue; }
        let saved = bodies.query_row([id], |r| r.get::<_, String>(0)).optional()?
            .ok_or_else(|| format!("{path} 在紙間有未協調變更或缺少可驗證共同版本。"))?;
        let new_path = if *id == target { next } else { path };
        let disk_body = disk.get(new_path).ok_or_else(|| format!("{new_path} 缺少磁碟檔，請先處理其他外部變更。"))?;
        if *id == target && saved != *disk_body
            && (!has_identity_text(&saved) || disk.keys().any(|path| path != next && !mapped.contains(path.as_str())))
        {
            return Err("搬移筆記改了連結時，須有可辨認的保留正文，且磁碟上只有一篇未配對的 Markdown。".into());
        }
        let links = link_changes(&saved, path, &prior, disk_body, new_path, &following, *id, target)
            .ok_or_else(|| format!("{path} 有未驗證的正文或連結變更。確認外部工具已更新可解析的目標連結，並先處理其他衝突。"))?;
        if links.count != 0 {
            edits.push(Edit { id: *id, path: path.clone(), body: disk_body.clone(), links: links.count, blocks: links.blocks });
        }
        revisions.push((*id, path.clone(), *version));
    }
    let canvases = plan_vault_relocation(conn, root, &old, next)?;
    verify_block_destinations(target, &after, &disk, &edits, &canvases)?;
    Ok(Relocation { target, root, directory: directory.into(), old, next: next.into(), revisions, edits, canvases })
}

impl Notebook {
    pub(crate) fn vault_relocation_origin(&self, note: i64) -> Result<(i64, String)> {
        origin(&self.connection, note)
    }

    pub(crate) fn preview_vault_relocation(&self, target: i64, directory: &Path, next: &str) -> Result<Relocation> {
        let tx = self.connection.unchecked_transaction()?;
        let preview = preflight(&tx, target, directory, next)?;
        tx.commit()?;
        Ok(preview)
    }

    pub(crate) fn apply_vault_relocation(&mut self, plan: &Relocation, backup: &Path) -> Result<()> {
        if self.preview_vault_relocation(plan.target, &plan.directory, &plan.next)? != *plan {
            return Err("紙間或磁碟資料已變更，請重新預覽。".into());
        }
        self.backup(backup)?;
        self.commit_vault_relocation(plan)
            .map_err(|error| format!("{error}　備份已保留：{}", backup.display()).into())
    }

    fn commit_vault_relocation(&mut self, plan: &Relocation) -> Result<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if preflight(&tx, plan.target, &plan.directory, &plan.next)? != *plan {
            return Err("紙間或磁碟資料已變更，請重新預覽。".into());
        }
        for canvas in &plan.canvases {
            if tx.execute(
                "UPDATE saved_canvases SET source=?3 WHERE id=?1 AND source=?2",
                params![canvas.id, canvas.original, canvas.revised],
            )? != 1 {
                return Err("Canvas 已更改或移除，請重新預覽。".into());
            }
        }
        tx.execute("UPDATE vault_paths SET relative_path=?2 WHERE note_id=?1", params![plan.target, plan.next])?;
        for edit in &plan.edits {
            tx.execute("UPDATE notes SET body=?2,version=version+1 WHERE id=?1", params![edit.id, edit.body])?;
            tx.execute("UPDATE vault_sync_base SET body=?2 WHERE note_id=?1", params![edit.id, edit.body])?;
        }
        super::super::obsidian_base_import::scope::mapped(&tx,plan.root,plan.target)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn mapped(db: &Notebook, root: i64, path: &str) -> Result<i64> {
        Ok(db.connection.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            (root, path), |r| r.get(0),
        )?)
    }

    #[test]
    fn external_move_updates_only_verified_link_destinations_and_recovers_from_backup() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let database = dir.join("live.sqlite");
        let backup = dir.join("backup.sqlite");
        let old = "片場/鏡頭.md";
        let next = "訪談/新鏡頭.md";
        let from = "分鏡/入口.md";
        let target_body = "\u{feff}# 取景\r\n原始位置的內容\r\n[[/支援.md|工作流程]]";
        let before = "\u{feff}---\r\naliases: [導覽]\r\n---\r\n先看 [[片場/鏡頭#取景|別名]] 再 [回看](../片場/鏡頭.md#取景) 或 [根路徑](片場/鏡頭#取景)。\r\n`[[片場/鏡頭]]` 和 [[找不到]] 不改。";
        let after = before.replace("[[片場/鏡頭#取景|別名]]", "[[訪談/新鏡頭#取景|別名]]")
            .replace("(../片場/鏡頭.md#取景)", "(../訪談/新鏡頭.md#取景)")
            .replace("(片場/鏡頭#取景)", "(訪談/新鏡頭#取景)");
        for folder in ["片場", "訪談", "分鏡"] {
            fs::create_dir_all(vault.join(folder))?;
        }
        fs::write(vault.join(old), target_body)?;
        fs::write(vault.join(from), before)?;
        fs::write(vault.join("支援.md"), "工作流程")?;
        let mut db = Notebook::open(&database, false)?;
        let root = db.import_vault("匯入批次", crate::vault::read(&vault)?.pages)?;
        let target = mapped(&db, root, old)?;
        let from_id = mapped(&db, root, from)?;
        let before_meta = db.note(target)?.unwrap();
        fs::rename(vault.join(old), vault.join(next))?;
        fs::write(vault.join(from), &after)?;
        let plan = db.preview_vault_relocation(target, &vault, next)?;
        assert!(plan.report().contains("1 篇筆記中已核對的 3 個連結"));
        db.apply_vault_relocation(&plan, &backup)?;
        assert!(!db.preview_vault_pull(root, &vault)?.actionable());
        drop(db);

        let current = Notebook::open(&database, false)?;
        let old_db = Notebook::open(&backup, false)?;
        assert_eq!(mapped(&current, root, next)?, target);
        assert!(mapped(&current, root, old).is_err());
        assert_eq!(mapped(&old_db, root, old)?, target);
        assert_eq!(mapped(&current, root, from)?, from_id);
        let target_note = current.note(target)?.unwrap();
        assert_eq!(target_note.body, target_body);
        assert_eq!(target_note.row.parent, before_meta.row.parent);
        assert_eq!(target_note.row.title, before_meta.row.title);
        let source = current.note(from_id)?.unwrap();
        assert_eq!(source.body, after);
        assert_eq!(old_db.note(from_id)?.unwrap().body, before);
        let links = current.vault_note_links(&source)?;
        assert_eq!(links.len(), 3);
        assert_eq!((links[0].note_id, links[0].anchor.as_deref(), links[0].label.as_str()),
                   (target, Some("#取景"), "別名"));
        for link in &links[1..] {
            assert_eq!((link.note_id, link.anchor.as_deref(), link.anchor_syntax),
                       (target, Some("#取景"), crate::markdown::AnchorSyntax::Markdown));
        }
        assert_eq!(fs::read(vault.join(from))?, after.as_bytes());
        assert_eq!(fs::read(vault.join(next))?, target_body.as_bytes());
        assert!(!vault.join(old).exists());
        drop((current, old_db));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn stale_preview_canvas_and_unrelated_changes_leave_both_sides_intact() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let backup = dir.join("stale.sqlite");
        let old = "原稿.md";
        let next = "採訪.md";
        let body = "不同於其他檔案的原文";
        let old_link = "[[/原稿|保留別名]]";
        let new_link = "[[/採訪|保留別名]]";
        fs::create_dir(&vault)?;
        fs::write(vault.join(old), body)?;
        fs::write(vault.join("入口.md"), old_link)?;
        let mut db = Notebook::open(&dir.join("database.sqlite"), false)?;
        let root = db.import_vault("本機", crate::vault::read(&vault)?.pages)?;
        let target = mapped(&db, root, old)?;
        let from = mapped(&db, root, "入口.md")?;
        assert!(db.preview_vault_relocation(target, &vault, "原稿.MD").is_err());
        assert!(db.preview_vault_relocation(target, &vault, "../outside.md").is_err());
        fs::write(vault.join(next), body)?;
        assert!(db.preview_vault_relocation(target, &vault, next).is_err());
        fs::remove_file(vault.join(old))?;
        // An external rename which did not update the link would break it.
        assert!(db.preview_vault_relocation(target, &vault, next).is_err());
        fs::write(vault.join("入口.md"), "[[/採訪|修改別名]]")?;
        assert!(db.preview_vault_relocation(target, &vault, next).is_err());
        fs::write(vault.join("入口.md"), new_link)?;
        fs::write(vault.join("第二候選.md"), body)?;
        assert!(db.preview_vault_relocation(target, &vault, next).is_err());
        fs::remove_file(vault.join("第二候選.md"))?;
        let plan = db.preview_vault_relocation(target, &vault, next)?;
        fs::write(vault.join("入口.md"), format!("{new_link}\n另一筆外部變更"))?;
        assert!(db.apply_vault_relocation(&plan, &backup).is_err());
        fs::write(vault.join("入口.md"), new_link)?;

        let canvas = crate::canvas::parse(serde_json::json!({"nodes":[{
            "id":"unknown","type":"file","file":next,"x":0,"y":0,"width":200,"height":140
        }]}).to_string())?;
        let canvas = db.save_canvas("Canvas 綁定", &canvas)?;
        db.set_canvas_vault(canvas, Some(root))?;
        let error = db.preview_vault_relocation(target, &vault, next).err().unwrap();
        assert!(error.to_string().contains("已指向新路徑"));
        assert!(db.apply_vault_relocation(&plan, &backup).is_err());
        db.set_canvas_vault(canvas, None)?;
        let note = db.note(from)?.unwrap();
        db.save(from, note.version, &note.row.title, "尚未協調的紙間改動")?;
        assert!(db.apply_vault_relocation(&plan, &backup).is_err());
        assert!(!backup.exists());
        assert_eq!(db.note(from)?.unwrap().body, "尚未協調的紙間改動");
        assert_eq!(mapped(&db, root, old)?, target);
        assert_eq!(fs::read_to_string(vault.join(next))?, body);
        assert_eq!(fs::read_to_string(vault.join("入口.md"))?, new_link);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }


    #[test]
    fn canvas_nodes_follow_two_relocations_with_exact_backup_and_no_disk_canvas_write() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let database = dir.join("work.sqlite");
        let old = "現場/鏡頭.md";
        let first_path = "訪談/新鏡頭.md";
        let second_path = "完成/鏡頭定稿.md";
        let original = concat!(
            "\u{feff}",
            r##"{
              "nodes":[
                {"id":"one","type":"file","file":"現場/鏡頭.md","subpath":"#片段","label":"甲","x":0,"y":1,"width":200,"height":140,"vendor":{"comment":"甲"}},
                {"id":"two","type":"file","file":"./現場/鏡頭.md","subpath":"#另一段","x":30,"y":-2,"width":300,"height":140},
                {"id":"fixed","type":"file","file":"無關.md","x":27,"y":8,"width":200,"height":170}
              ],
              "edges":[{"id":"edge","fromNode":"one","toNode":"fixed","label":"要保留"}],
              "vendor":{"number":12345678901234567890123456789,"tags":["甲","乙"]}
            }"##,
        );
        let external = original.replace("\"x\":0", "\"x\":999");
        let paths = [old, first_path, second_path];
        for part in ["現場", "訪談", "完成"] {
            fs::create_dir_all(vault.join(part))?;
        }
        fs::write(vault.join(old), "# 片段\n原文\n# 另一段\n測試")?;
        fs::write(vault.join("無關.md"), "不動")?;
        fs::write(vault.join("入口.md"), format!("[[/{old}#片段|連回]]"))?;
        fs::write(vault.join("畫布.canvas"), original)?;
        let mut db = Notebook::open(&database, false)?;
        let root = db.import_vault("甲", crate::vault::read(&vault)?.pages)?;
        let other = db.import_vault("乙", crate::vault::read(&vault)?.pages)?;
        let target = mapped(&db, root, old)?;
        let entry = mapped(&db, root, "入口.md")?;
        let canvas = db.save_canvas("畫布", &crate::canvas::read(&vault.join("畫布.canvas"))?)?;
        let unselected = db.save_canvas("另一庫", &crate::canvas::read(&vault.join("畫布.canvas"))?)?;
        let irrelevant = db.save_canvas("無節點", &crate::canvas::parse("{ }".into())?)?;
        db.set_canvas_vault(canvas, Some(root))?;
        db.set_canvas_vault(unselected, Some(other))?;
        db.set_canvas_vault(irrelevant, Some(root))?;
        fs::write(vault.join("畫布.canvas"), &external)?;

        for (step, pair) in paths.windows(2).enumerate() {
            let previous = db.read_canvas(canvas)?;
            fs::rename(vault.join(pair[0]), vault.join(pair[1]))?;
            fs::write(vault.join("入口.md"), format!("[[/{}#片段|連回]]", pair[1]))?;
            let plan = db.preview_vault_relocation(target, &vault, pair[1])?;
            assert!(plan.report().contains("更新 1 張 Canvas 的 2 個檔案節點"));
            let backup = dir.join(format!("before-{step}.sqlite"));
            if step == 0 {
                let updated = db.edit_canvas(canvas, &previous, super::super::super::canvas_edit::CanvasEdit::Geometry {
                    id:"one", geometry:[3,4,200,140],
                })?;
                let stale = db.apply_vault_relocation(&plan, &backup);
                assert!(stale.is_err() && !backup.exists());
                assert_eq!(db.read_canvas(canvas)?.as_str(), updated.as_str());
                let fresh = db.preview_vault_relocation(target, &vault, pair[1])?;
                let interrupted = dir.join("rollback.sqlite");
                db.connection.execute_batch(&format!(
                    "CREATE TEMP TRIGGER fail_link_update BEFORE UPDATE OF body ON notes
                     WHEN OLD.id={entry} BEGIN SELECT RAISE(ABORT,'test atomic rollback'); END;"
                ))?;
                assert!(db.apply_vault_relocation(&fresh, &interrupted).is_err());
                db.connection.execute_batch("DROP TRIGGER fail_link_update;")?;
                assert_eq!(db.read_canvas(canvas)?.as_str(), updated.as_str());
                assert_eq!(mapped(&db, root, old)?, target);
                assert_eq!(db.note(entry)?.unwrap().body, format!("[[/{old}#片段|連回]]"));
                let rolled_back = Notebook::open(&interrupted, false)?;
                assert_eq!(rolled_back.read_canvas(canvas)?.as_str(), updated.as_str());
            }

            let current = db.read_canvas(canvas)?;
            let plan = db.preview_vault_relocation(target, &vault, pair[1])?;
            db.apply_vault_relocation(&plan, &backup)?;
            let restored = Notebook::open(&backup, false)?;
            assert_eq!(restored.read_canvas(canvas)?.as_str(), current.as_str());
            assert_eq!(mapped(&restored, root, pair[0])?, target);
            let mut expected = current.json().clone();
            expected["nodes"][0]["file"] = pair[1].into();
            expected["nodes"][1]["file"] = pair[1].into();
            assert_eq!(*db.read_canvas(canvas)?.json(), expected);
            assert_eq!(db.read_canvas(unselected)?.as_str(), original);
            assert_eq!(db.read_canvas(irrelevant)?.as_str(), "{ }");
            assert_eq!(mapped(&db, root, pair[1])?, target);
            let link = db.resolve_canvas_file_link(canvas, Some(root), "one")?;
            assert_eq!((link.note_id, link.subpath.as_deref()), (Some(target), Some("#片段")));
            assert!(!db.preview_vault_pull(root, &vault)?.actionable());
            assert_eq!(fs::read(vault.join("畫布.canvas"))?, external.as_bytes());
        }
        drop(db);
        let db = Notebook::open(&database, false)?;
        let link = db.resolve_canvas_file_link(canvas, Some(root), "two")?;
        assert_eq!((link.note_id, link.file.as_str(), link.subpath.as_deref()),
                   (Some(target), second_path, Some("#另一段")));
        assert_eq!(fs::read(vault.join("畫布.canvas"))?, external.as_bytes());
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }


    #[test]
    fn external_relocations_keep_block_targets_canvas_geometry_and_exact_backups() -> Result<()> {
        use crate::markdown::block_anchors::{BlockKind, BlockLookup, resolve_block};

        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let database = dir.join("work.sqlite");
        let paths = ["甲棚/鏡頭.md", "乙棚/新鏡頭.md", "完成/鏡頭定稿.md"];
        let original = "🎬 原區塊 **保留** ^Shot-A\r\n\r\n- 旁邊的清單 ^other";
        let caller = |path: &str| {
            format!("[[/{path}#^Shot-A|別名]] [Markdown](./{path}#^Shot-A) [[失聯#^missing]] `[[/甲棚/鏡頭.md#^unknown]]`")
        };
        for part in ["甲棚", "乙棚", "完成"] {
            fs::create_dir_all(vault.join(part))?;
        }
        fs::write(vault.join(paths[0]), original)?;
        fs::write(vault.join("入口.md"), caller(paths[0]))?;
        let canvas = serde_json::json!({
            "nodes":[
                {"id":"shot","type":"file","file":paths[0],"subpath":"#^Shot-A","x":-10,"y":42,"width":200,"height":160,"vendor":"retained"},
                {"id":"list","type":"file","file":"./甲棚/鏡頭.md","subpath":"#^other","x":250,"y":5,"width":200,"height":160}
            ],
            "edges":[{"id":"edge","fromNode":"shot","toNode":"list","label":"保留"}]
        });
        let disk_canvas = serde_json::to_string_pretty(&canvas)?;
        fs::write(vault.join("來源.canvas"), &disk_canvas)?;
        let mut db = Notebook::open(&database, false)?;
        let root = db.import_vault("合成來源", crate::vault::read(&vault)?.pages)?;
        let target = mapped(&db, root, paths[0])?;
        let entry = mapped(&db, root, "入口.md")?;
        let canvas_id = db.save_canvas("來源畫布", &crate::canvas::read(&vault.join("來源.canvas"))?)?;
        db.set_canvas_vault(canvas_id, Some(root))?;

        for (step, pair) in paths.windows(2).enumerate() {
            let before = db.read_canvas(canvas_id)?;
            let next = pair[1];
            fs::rename(vault.join(pair[0]), vault.join(next))?;
            fs::write(vault.join("入口.md"), caller(next))?;
            let plan = db.preview_vault_relocation(target, &vault, next)?;
            assert!(plan.report().contains("已核對 2 個筆記區塊連結及 2 個 Canvas 區塊參照"));
            let backup = dir.join(format!("backup-{step}.sqlite"));
            if step == 0 {
                fs::write(vault.join(next), format!("{original}\r\n\r\n變更後重複 ^Shot-A"))?;
                assert!(db.apply_vault_relocation(&plan, &backup).is_err());
                assert!(!backup.exists());
                assert_eq!(mapped(&db, root, pair[0])?, target);
                assert_eq!(db.read_canvas(canvas_id)?.as_str(), before.as_str());
                fs::write(vault.join(next), original)?;
                fs::write(vault.join("入口.md"), caller(next).replace("#^Shot-A", "#^other"))?;
                assert!(db.apply_vault_relocation(&plan, &backup).is_err());
                assert!(!backup.exists());
                fs::write(vault.join("入口.md"), caller(next))?;
            }
            db.apply_vault_relocation(&plan, &backup)?;
            let previous = Notebook::open(&backup, false)?;
            assert_eq!(mapped(&previous, root, pair[0])?, target);
            assert_eq!(previous.note(entry)?.unwrap().body, caller(pair[0]));
            assert_eq!(previous.read_canvas(canvas_id)?.as_str(), before.as_str());
            assert!(!db.preview_vault_pull(root, &vault)?.actionable());
            let live_canvas = db.read_canvas(canvas_id)?;
            let mut expected = canvas.clone();
            expected["nodes"][0]["file"] = next.into();
            expected["nodes"][1]["file"] = next.into();
            assert_eq!(*live_canvas.json(), expected);
            assert_eq!(fs::read(vault.join(next))?, original.as_bytes());
            assert_eq!(fs::read(vault.join("入口.md"))?, caller(next).as_bytes());
            assert_eq!(fs::read(vault.join("來源.canvas"))?, disk_canvas.as_bytes());
        }
        drop(db);
        let db = Notebook::open(&database, false)?;
        let from = db.note(entry)?.unwrap();
        let links = db.vault_note_links(&from)?;
        assert_eq!(links.len(), 2);
        for link in links {
            assert_eq!((link.note_id, link.anchor.as_deref()), (target, Some("#^Shot-A")));
            let content = db.note(link.note_id)?.unwrap().body;
            let BlockLookup::Found { source, kind: BlockKind::Paragraph } =
                resolve_block(&content, link.anchor.as_deref().unwrap()) else { panic!("block not found"); };
            assert_eq!(&content[source], "🎬 原區塊 **保留**");
        }
        let node = db.resolve_canvas_file_link(canvas_id, Some(root), "shot")?;
        assert_eq!((node.note_id, node.subpath.as_deref(), node.file.as_str()),
                   (Some(target), Some("#^Shot-A"), paths[2]));
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn broken_block_references_prevent_relocation_even_with_matching_note_bytes() -> Result<()> {
        use serde_json::json;

        for (anchor, body, reason, in_canvas) in [
            ("gone", "其他區塊 ^used", "找不到這個區塊 ID", false),
            ("dup", "甲 ^dup\r\n\r\n- 乙 ^dup", "有重複的區塊 ID", false),
            ("bad_id", "原稿 ^bad_id", "使用了無效的區塊 ID", false),
            ("dup", "甲 ^dup\r\n\r\n乙 ^dup", "有重複的區塊 ID", true),
        ] {
            let dir = super::super::super::tests::data_dir();
            let vault = dir.join("Vault");
            let backup = dir.join("unused.sqlite");
            let old = "舊檔.md";
            let next = "新檔.md";
            let reference = |path| format!("[[/{path}#^{anchor}|段落]]");
            let before = if in_canvas { "獨立入口".to_owned() } else { reference(old) };
            fs::create_dir(&vault)?;
            fs::write(vault.join(old), body)?;
            fs::write(vault.join("入口.md"), &before)?;
            let mut db = Notebook::open(&dir.join("work.sqlite"), false)?;
            let root = db.import_vault("合成來源", crate::vault::read(&vault)?.pages)?;
            let target = mapped(&db, root, old)?;
            let canvas = if in_canvas {
                let source = json!({"nodes":[{
                   "id":"node-keep","type":"file","file":old,"subpath":format!("#^{anchor}"),
                   "x":0,"y":0,"width":100,"height":100
                }]}).to_string();
                let id = db.save_canvas("選區畫布", &crate::canvas::parse(source.clone())?)?;
                db.set_canvas_vault(id, Some(root))?;
                Some((id, source))
            } else {
                None
            };

            fs::rename(vault.join(old), vault.join(next))?;
            if !in_canvas { fs::write(vault.join("入口.md"), reference(next))?; }
            let error = db.preview_vault_relocation(target, &vault, next).err().unwrap().to_string();
            assert!(error.contains(reason) && error.contains(&format!("#^{anchor}")), "{error}");
            assert_eq!(mapped(&db, root, old)?, target);
            assert_eq!(db.note(mapped(&db, root, "入口.md")?)?.unwrap().body, before);
            assert!(!backup.exists());
            assert_eq!(fs::read(vault.join(next))?, body.as_bytes());
            if let Some((id, source)) = canvas {
                assert!(error.contains("node-keep"), "{error}");
                assert_eq!(db.read_canvas(id)?.as_str(), source);
            }
            drop(db);
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }


    #[test]
    fn external_move_rebases_only_relative_links_to_the_same_exact_note_and_block() -> Result<()> {
        use crate::markdown::block_anchors::{BlockLookup, resolve_block};

        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let database = dir.join("work.sqlite");
        let paths = ["甲/鏡頭.md", "乙/子層/新鏡頭.md", "定稿/鏡頭.md"];
        let relative = [".", "../../甲", "../甲"];
        let destination = "原區塊 ^Shot-A";
        let decoy = "同名他篇 ^Shot-A";
        let authored = |base: &str| format!(
            "🎬 鏡頭原文\r\n\r\n可辨識來源 ^Own\r\n\r\n[Markdown]({base}/細節.md#^Shot-A) [[{base}/細節.md#^Shot-A|Wiki]]"
        );
        let incoming = |path: &str| format!("[[/{path}#^Own|傳入段落]]");
        for folder in ["甲", "乙/子層", "定稿"] { fs::create_dir_all(vault.join(folder))?; }
        fs::write(vault.join(paths[0]), authored(relative[0]))?;
        fs::write(vault.join("入口.md"), incoming(paths[0]))?;
        fs::write(vault.join("甲/細節.md"), destination)?;
        fs::write(vault.join("乙/子層/細節.md"), decoy)?;
        let mut db = Notebook::open(&database, false)?;
        let root = db.import_vault("來源", crate::vault::read(&vault)?.pages)?;
        let moving = mapped(&db, root, paths[0])?;
        let detail = mapped(&db, root, "甲/細節.md")?;
        let extra = vault.join("無關草稿.md");
        for (step, pair) in paths.windows(2).enumerate() {
            let next = pair[1];
            let backup = dir.join(format!("backup-{step}.sqlite"));
            fs::rename(vault.join(pair[0]), vault.join(next))?;
            fs::write(vault.join("入口.md"), incoming(next))?;
            // The old explicit relative path now finds a different same-named note
            // at the first destination, or no matching note during the next move.
            assert!(db.preview_vault_relocation(moving, &vault, next).is_err());
            fs::write(vault.join(next), authored(relative[step + 1]))?;
            if step == 0 {
                fs::write(&extra, "多出另一篇未配對檔")?;
                assert!(db.preview_vault_relocation(moving, &vault, next).is_err());
                fs::remove_file(&extra)?;
            }
            let plan = db.preview_vault_relocation(moving, &vault, next)?;
            assert!(plan.report().contains("已核對搬移筆記的 2 個相對傳出連結"));
            assert!(plan.report().contains("已核對 3 個筆記區塊連結及 0 個 Canvas 區塊參照"));
            fs::write(vault.join(next), authored(relative[step + 1]).replace("鏡頭原文", "非連結改動"))?;
            assert!(db.apply_vault_relocation(&plan, &backup).is_err());
            assert!(!backup.exists());
            fs::write(vault.join(next), authored(relative[step + 1]))?;
            db.apply_vault_relocation(&plan, &backup)?;
            let old_db = Notebook::open(&backup, false)?;
            assert_eq!(mapped(&old_db, root, pair[0])?, moving);
            assert_eq!(old_db.note(moving)?.unwrap().body, authored(relative[step]));
            assert_eq!(mapped(&db, root, next)?, moving);
            assert!(!db.preview_vault_pull(root, &vault)?.actionable());
            assert_eq!(fs::read_to_string(vault.join(next))?, authored(relative[step + 1]));
            assert_eq!(fs::read_to_string(vault.join("入口.md"))?, incoming(next));
        }
        drop(db);
        let db = Notebook::open(&database, false)?;
        let page = db.note(moving)?.unwrap();
        let links = db.vault_note_links(&page)?;
        assert_eq!(links.len(), 2);
        for link in links {
            assert_eq!((link.note_id, link.anchor.as_deref()), (detail, Some("#^Shot-A")));
            let note = db.note(link.note_id)?.unwrap();
            let BlockLookup::Found { source, .. } = resolve_block(&note.body, link.anchor.as_deref().unwrap())
                else { panic!("missing referenced block") };
            assert_eq!(&note.body[source], "原區塊");
        }
        assert_eq!(fs::read_to_string(vault.join("甲/細節.md"))?, destination);
        assert_eq!(fs::read_to_string(vault.join("乙/子層/細節.md"))?, decoy);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn external_rebase_refuses_an_outgoing_block_id_duplicated_in_its_destination() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("Vault");
        fs::create_dir_all(vault.join("甲"))?;
        fs::create_dir_all(vault.join("乙"))?;
        let source = "共同正文 [到](./細節.md#^duplicate)";
        let destination = "第一段 ^duplicate\n\n第二段 ^duplicate";
        fs::write(vault.join("甲/移動.md"), source)?;
        fs::write(vault.join("甲/細節.md"), destination)?;
        let mut db = Notebook::open(&dir.join("work.sqlite"), false)?;
        let root = db.import_vault("來源", crate::vault::read(&vault)?.pages)?;
        let move_id = mapped(&db, root, "甲/移動.md")?;
        fs::rename(vault.join("甲/移動.md"), vault.join("乙/新稿.md"))?;
        let after = source.replace("./細節.md", "../甲/細節.md");
        fs::write(vault.join("乙/新稿.md"), &after)?;
        let error = db.preview_vault_relocation(move_id, &vault, "乙/新稿.md").err().unwrap().to_string();
        assert!(error.contains("#^duplicate") && error.contains("有重複的區塊 ID"), "{error}");
        assert_eq!(mapped(&db, root, "甲/移動.md")?, move_id);
        assert_eq!(db.note(move_id)?.unwrap().body, source);
        assert_eq!(fs::read_to_string(vault.join("乙/新稿.md"))?, after);
        assert_eq!(fs::read_to_string(vault.join("甲/細節.md"))?, destination);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn skipped_link_cannot_disguise_an_occupied_retired_path() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let vault = dir.join("Vault");
        fs::create_dir_all(vault.join("舊位置"))?;
        fs::write(vault.join("舊位置/舊檔.md"), "私人測試標記")?;
        let mut db = Notebook::open(&dir.join("test.sqlite"), false)?;
        let root = db.import_vault("本機", crate::vault::read(&vault)?.pages)?;
        let id = mapped(&db, root, "舊位置/舊檔.md")?;
        fs::rename(vault.join("舊位置/舊檔.md"), vault.join("新檔.md"))?;
        std::os::unix::fs::symlink(vault.join("新檔.md"), vault.join("舊位置/舊檔.md"))?;
        assert!(db.preview_vault_relocation(id, &vault, "新檔.md").is_err());
        fs::remove_file(vault.join("舊位置/舊檔.md"))?;
        fs::remove_dir(vault.join("舊位置"))?;
        std::os::unix::fs::symlink(&vault, vault.join("舊位置"))?;
        assert!(db.preview_vault_relocation(id, &vault, "新檔.md").is_err());
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
