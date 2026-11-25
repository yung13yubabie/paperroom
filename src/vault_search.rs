//! Query current notebook bodies using the original imported Vault paths.
use super::{Notebook, Result};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use rusqlite::{Row, params};
use std::collections::{BTreeMap, BTreeSet};

const MAX_FRONTMATTER: usize = 64 * 1024;
pub(super) const MAX_SCAN: usize = 128 * 1024 * 1024;
const MAX_FILTER: usize = 4096;
const MAX_TAG_NODES: usize = 50_000;
const MAX_TAG_BYTES: usize = 4 * 1024 * 1024;

pub(crate) fn frontmatter(body: &str) -> (Option<&str>, &str) {
    let body=body.strip_prefix('\u{feff}').unwrap_or(body);
    let Some((line,rest))=body.split_once('\n') else{return (None,body)};
    if line.trim_end_matches('\r')!="---" {return (None,body);}
    let mut offset=0;
    for line in rest.split_inclusive('\n') {
        if matches!(line.trim_end_matches(['\r','\n']),"---"|"...") {
            return ((offset<=MAX_FRONTMATTER).then(||&rest[..offset]), &rest[offset+line.len()..]);
        }
        offset+=line.len();
    }
    (None,body)
}

fn scalar(raw:&str)->Option<String>{
    let s=raw.trim();
    if s.is_empty() {return None;}
    if s.starts_with('"') {return serde_json::from_str::<String>(s).ok();}
    if s.starts_with('\'') {return s.strip_prefix('\'')?.strip_suffix('\'').map(|v|v.replace("''","'"));}
    if s.starts_with(['&','*','!','|','>','{','[','#']) {return None;}
    Some(s.split_once(" #").map_or(s,|(text,_)|text).trim_end().into())
}

// Commas inside quoted aliases are part of the alias, not list separators.
fn values(raw:&str, mut check:impl FnMut(&str)) {
    let s=raw.trim();
    if let Some(list)=s.strip_prefix('[').and_then(|v|v.strip_suffix(']')) {
        let mut quote=None;
        let mut escape=false;
        let mut start=0;
        for (i,c) in list.char_indices() {
            if escape {escape=false;continue;}
            match quote {
                Some('"') if c=='\\' => escape=true,
                Some(q) if q==c => quote=None,
                None if matches!(c,'\''|'"') => quote=Some(c),
                None if c==',' => {if let Some(v)=scalar(&list[start..i]){check(&v);} start=i+1;}
                _=>{}
            }
        }
        if quote.is_none() && let Some(v)=scalar(&list[start..]) {check(&v);}
    } else if let Some(v)=scalar(s) {check(&v);}
}

fn tag_char(c:char)->bool {c.is_alphanumeric() || matches!(c,'_'|'-'|'/')}
fn valid_tag_path(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_FILTER
        && s.chars().all(tag_char)
        && s.split('/').all(|part| !part.is_empty())
}

fn valid_tag(s: &str) -> bool {
    valid_tag_path(s) && s.chars().any(|c| !c.is_numeric())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TagMatch {
    Exact,
    Descendants,
}

pub(super) struct TagFilter {
    key: String,
    mode: TagMatch,
}

impl TagFilter {
    pub(super) fn parse(raw: &str, mode: TagMatch) -> Result<Self> {
        let raw = raw.trim();
        let tag = raw.strip_prefix('#').unwrap_or(raw);
        let valid = match mode {
            TagMatch::Exact => valid_tag(tag),
            TagMatch::Descendants => valid_tag_path(tag),
        };
        if !valid {
            return Err("標籤名稱無效。請輸入完整標籤，例如 #拍攝/夜景。".into());
        }
        Ok(Self { key: tag.to_lowercase(), mode })
    }

    fn matches(&self, label: &str) -> bool {
        self.matches_normalized(&label.to_lowercase())
    }

    fn matches_normalized(&self, tag: &str) -> bool {
        tag == self.key
            || (matches!(self.mode, TagMatch::Descendants)
                && tag.strip_prefix(&self.key).is_some_and(|rest| rest.starts_with('/')))
    }
}

fn visit_inline_tags(body: &str, visit: &mut impl FnMut(&str) -> Result<bool>) -> Result<bool> {
    let mut skip = 0usize;
    for (event, range) in Parser::new_ext(body, Options::ENABLE_WIKILINKS).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_) | Tag::Image { .. } | Tag::Link { .. }) => skip += 1,
            Event::End(TagEnd::CodeBlock | TagEnd::Image | TagEnd::Link) => {
                skip = skip.saturating_sub(1);
            }
            Event::Text(_) if skip == 0 => {
                let text = &body[range.clone()];
                for (at, _) in text.match_indices('#') {
                    let before = &body[..range.start + at];
                    if before.chars().next_back().is_some_and(|c| {
                        tag_char(c) || matches!(c, '#' | '\\' | '=' | '?')
                    }) {
                        continue;
                    }
                    let tail = &text[at + 1..];
                    let tag = &tail[..tail.find(|c: char| !tag_char(c)).unwrap_or(tail.len())];
                    if valid_tag(tag) && visit(tag)? {
                        return Ok(true);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

pub(super) fn visit_property(
    meta: Option<&str>,
    wanted: &str,
    mut visit: impl FnMut(&str) -> Result<bool>,
) -> Result<bool> {
    let mut key = "";
    for line in meta.into_iter().flat_map(str::lines) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let raw = if let Some(item) = trimmed.strip_prefix("- ") {
            item
        } else if line.len() == trimmed.len() {
            let Some((name, value)) = line.split_once(':') else {
                key = "";
                continue;
            };
            key = name.trim_end();
            value
        } else {
            key = "";
            continue;
        };
        if key == wanted {
            let mut result = Ok(false);
            values(raw, |v| {
                if matches!(result, Ok(false)) {
                    result = visit(v);
                }
            });
            if result? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn visit_tags(
    meta: Option<&str>,
    prose: &str,
    mut visit: impl FnMut(&str) -> Result<bool>,
) -> Result<bool> {
    if visit_property(meta, "tags", |v| {
        let v = v.strip_prefix('#').unwrap_or(v);
        if valid_tag(v) { visit(v) } else { Ok(false) }
    })? {
        return Ok(true);
    }
    visit_inline_tags(prose, &mut visit)
}

pub(super) fn matches_alias(body: &str, alias: &str) -> Result<bool> {
    visit_property(frontmatter(body).0, "aliases", |v| Ok(v == alias))
}

pub(crate) fn vault_aliases_preview(body: &str) -> Result<Vec<String>> {
    let mut aliases = Vec::new();
    visit_property(frontmatter(body).0, "aliases", |value| {
        let alias = value.chars().filter(|c| !c.is_control()).take(80).collect::<String>();
        if !alias.trim().is_empty() && !aliases.contains(&alias) {
            aliases.push(alias);
        }
        Ok(aliases.len() == 3)
    })?;
    Ok(aliases)
}

pub(super) fn matches_tag(body: &str, tag: &TagFilter) -> Result<bool> {
    let (meta, prose) = frontmatter(body);
    visit_tags(meta, prose, |v| Ok(tag.matches(v)))
}

#[derive(Default)]
pub(crate) struct TagBranch {
    pub(crate) name: String,
    pub(crate) exact: bool,
    pub(crate) note_count: usize,
    pub(crate) exact_count: usize,
    parent: usize,
    last_note: Option<i64>,
    last_exact_note: Option<i64>,
    pub(crate) children: BTreeMap<String, usize>,
}

pub(crate) struct VaultTagTree {
    pub(crate) branches: Vec<TagBranch>,
    label_bytes: usize,
}

impl VaultTagTree {
    fn new() -> Self {
        Self { branches: vec![TagBranch::default()], label_bytes: 0 }
    }

    fn record(&mut self, note: i64, tag: &str) -> Result<()> {
        let mut parent = 0;
        for part in tag.split('/') {
            let key = part.to_lowercase();
            let child = match self.branches[parent].children.get(&key) {
                Some(&child) => child,
                None => {
                    self.label_bytes += part.len() + key.len();
                    if self.branches.len() >= MAX_TAG_NODES || self.label_bytes > MAX_TAG_BYTES {
                        return Err("標籤階層超過大小上限，請改用 Vault 來源搜尋。".into());
                    }
                    let child = self.branches.len();
                    self.branches.push(TagBranch { name: part.into(), parent, ..Default::default() });
                    self.branches[parent].children.insert(key, child);
                    child
                }
            };
            let branch = &mut self.branches[child];
            if branch.last_note != Some(note) {
                branch.note_count += 1;
                branch.last_note = Some(note);
            }
            parent = child;
        }
        let exact = &mut self.branches[parent];
        exact.exact = true;
        if exact.last_exact_note != Some(note) {
            exact.last_exact_note = Some(note);
            exact.exact_count += 1;
        }
        Ok(())
    }

    pub(crate) fn path(&self, mut branch: usize) -> String {
        let mut parts = Vec::new();
        while branch != 0 {
            let tag = &self.branches[branch];
            parts.push(tag.name.as_str());
            branch = tag.parent;
        }
        parts.reverse();
        parts.join("/")
    }
}

pub(crate) const MAX_TAG_TERMS: usize = 32;

#[path = "vault_tag_panel.rs"]
mod panel;
pub(crate) use panel::TagPanel;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VaultSourceItem {
    Folder(String),
    Note { id: i64, path: String },
}

impl Notebook {
    fn scan_vault_sources(
        &self,
        root: i64,
        folder: &str,
        with_body: bool,
        mut visit: impl FnMut(&Row<'_>, &str) -> Result<()>,
    ) -> Result<()> {
        let mut stmt = self.connection.prepare(
            "SELECT n.id,p.relative_path,CASE WHEN ?3 THEN n.body ELSE '' END
             FROM vault_paths p JOIN notes n ON n.id=p.note_id JOIN notes r ON r.id=p.import_root
             WHERE p.import_root=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL
             AND (?2='' OR instr(p.relative_path,?2||'/')=1) ORDER BY p.relative_path,n.id"
        )?;
        let mut rows = stmt.query(params![root, folder, with_body])?;
        let mut size = 0;
        while let Some(row) = rows.next()? {
            let body = row.get_ref(2)?.as_str()?;
            size += body.len();
            if size > MAX_SCAN {
                return Err("搜尋內容超過 128 MiB，請指定較小的來源資料夾。".into());
            }
            visit(row, body)?;
        }
        Ok(())
    }

    pub(crate) fn search_vault_sources(
        &self,
        root: i64,
        folder: &str,
        alias: &str,
        raw_tag: &str,
        mode: TagMatch,
    ) -> Result<Vec<(i64, String)>> {
        // Keep the blank request as whole-Vault search, but never rewrite a
        // nonblank path to select a similarly named imported folder.
        let folder = if folder.trim().is_empty() { "" } else { folder.trim_end_matches('/') };
        let alias = alias.trim();
        if [folder, alias].iter().any(|s| s.len() > MAX_FILTER || s.chars().any(char::is_control))
            || folder.contains(['\\', ':'])
            || (!folder.is_empty() && folder.split('/').any(|p| matches!(p, "" | "." | "..")))
        {
            return Err("請使用 Vault 相對資料夾路徑或完整別名。".into());
        }
        let tag = (!raw_tag.trim().is_empty()).then(|| TagFilter::parse(raw_tag, mode)).transpose()?;
        let mut hits = Vec::new();
        self.scan_vault_sources(root, folder, !alias.is_empty() || tag.is_some(), |row, body| {
            if !alias.is_empty() && !matches_alias(body, alias)? {
                return Ok(());
            }
            if let Some(filter) = &tag && !matches_tag(body, filter)? {
                return Ok(());
            }
            hits.push((row.get(0)?, row.get(1)?));
            Ok(())
        })?;
        Ok(hits)
    }

    pub(crate) fn search_vault_tag_terms(&self, root: i64, terms: &[(String, TagMatch)]) -> Result<Vec<(i64, String)>> {
        if terms.is_empty() || terms.len() > MAX_TAG_TERMS {
            return Err(format!("請選擇 1 到 {MAX_TAG_TERMS} 個標籤條件。").into());
        }
        let filters = terms.iter().map(|(tag, mode)| TagFilter::parse(tag, *mode))
            .collect::<Result<Vec<_>>>()?;
        let all = (1u64 << filters.len()) - 1;
        let mut hits = Vec::new();
        self.scan_vault_sources(root, "", true, |row, body| {
            let (meta, prose) = frontmatter(body);
            let mut matches = 0u64;
            if visit_tags(meta, prose, |value| {
                let normalized = value.to_lowercase();
                for (i, filter) in filters.iter().enumerate() {
                    if filter.matches_normalized(&normalized) { matches |= 1 << i; }
                }
                Ok(matches == all)
            })? {
                hits.push((row.get(0)?, row.get(1)?));
            }
            Ok(())
        })?;
        Ok(hits)
    }

    pub(crate) fn vault_source_children(&self, root: i64, folder: &str) -> Result<Vec<VaultSourceItem>> {
        let mut prefix = folder.to_owned();
        if !prefix.is_empty() {
            prefix.push('/');
        }
        let mut dirs = BTreeSet::new();
        let mut notes = Vec::new();
        self.scan_vault_sources(root, folder, false, |row, _| {
            let path = row.get_ref(1)?.as_str()?;
            let rest = path.strip_prefix(&prefix).ok_or("無法確認 Vault 來源路徑。")?;
            if let Some((dir, _)) = rest.split_once('/') {
                dirs.insert(format!("{prefix}{dir}"));
            } else {
                notes.push(VaultSourceItem::Note { id: row.get(0)?, path: path.to_owned() });
            }
            Ok(())
        })?;
        Ok(dirs.into_iter().map(VaultSourceItem::Folder).chain(notes).collect())
    }

    pub(crate) fn vault_tag_tree(&self, root: i64) -> Result<VaultTagTree> {
        let mut tree = VaultTagTree::new();
        self.scan_vault_sources(root, "", true, |row, body| {
            let id = row.get(0)?;
            let (meta, prose) = frontmatter(body);
            visit_tags(meta, prose, |tag| {
                tree.record(id, tag)?;
                Ok(false)
            })?;
            Ok(())
        })?;
        Ok(tree)
    }

    pub(crate) fn vault_search_hit(&self,root:i64,id:i64,path:&str)->Result<bool>{
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM vault_paths p JOIN notes n ON n.id=p.note_id
            JOIN notes r ON r.id=p.import_root WHERE p.import_root=?1 AND p.note_id=?2 AND p.relative_path=?3
            AND n.trash_root IS NULL AND r.trash_root IS NULL)", params![root,id,path],|r|r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vault_filters_intersect_on_one_stable_source() -> Result<()> {
        use crate::archive::ImportedPage;
        let page=|path:&str,body:&str|ImportedPage{parts:path.split('/').map(str::to_owned).collect(),title:"場景".into(),body:body.into()};
        let text="\u{feff}---\r\naliases: [燈光別名, \"甲, 乙\"]\r\ntags:\r\n  - 研究/夜景\r\n---\r\n開拍 #拍攝/Ready";
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let root=db.import_vault("專案",vec![page("片場/主棚/場景.md",text),page("片場二/場景.md",text),page("片場/場景.md","#拍攝/ready")])?;
        let other=db.import_vault("專案",vec![page("片場/主棚/場景.md",text)])?;
        let found=db.search_vault_sources(root,"片場","燈光別名","#拍攝/READY", TagMatch::Exact)?;
        assert_eq!(found.len(),1);
        let (id,path)=&found[0];
        assert_eq!(path,"片場/主棚/場景.md");
        assert_eq!(db.note(*id)?.unwrap().body,text);
        assert!(db.vault_search_hit(root,*id,path)?);
        assert!(!db.vault_search_hit(other,*id,path)?);
        assert_eq!(db.search_vault_sources(root,"片場","甲, 乙","研究/夜景", TagMatch::Exact)?,found);
        assert!(db.search_vault_sources(root,"片場","燈光","", TagMatch::Exact).is_ok_and(|v|v.is_empty()));
        assert!(db.search_vault_sources(root,"","","研究", TagMatch::Exact)?.is_empty());
        db.trash(*id)?;
        assert!(!db.vault_search_hit(root,*id,path)?);
        assert!(db.search_vault_sources(root,"片場","燈光別名","", TagMatch::Exact)?.is_empty());
        Ok(())
    }

    #[test]
    fn inline_tags_skip_code_links_and_escaped_hashes() -> Result<()> {
        let md = "---\naliases:\n- '道具'\nother: #remark\n---\n\n`#code` [跳轉](https://example.org/#url) \\#escaped\n\n```md\n#fenced\n```\n\n#真的";
        assert!(matches_alias(md, "道具")?);
        for tag in ["code", "url", "escaped", "fenced", "remark"] {
            assert!(!matches_tag(md, &TagFilter::parse(tag, TagMatch::Exact)?)?, "{tag}");
        }
        assert!(matches_tag(md, &TagFilter::parse("真的", TagMatch::Exact)?)?);
        Ok(())
    }

    #[test]
    fn nested_tag_tree_and_shortcut_follow_saved_imports_through_reopen_and_restore() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let vault = dir.join("合成 Vault");
        for folder in ["主棚", "外景", "通告"] {
            std::fs::create_dir_all(vault.join(folder))?;
        }
        let originals = [
            ("主棚/鏡頭.md", "---\naliases: [主角]\ntags: [拍攝/夜景, '2026/內景']\n---\n#拍攝/夜景 #拍攝/夜景/待剪"),
            ("外景/鏡頭.md", "#拍攝 #拍攝\n"),
            ("通告/道具.md", "#拍攝室 #剪輯/READY\n\n`#拍攝/假標籤`"),
        ];
        for (path, body) in originals {
            std::fs::write(vault.join(path), body)?;
        }

        let path = dir.join("notes.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let root = db.import_vault("合成 Vault", crate::vault::read(&vault)?.pages)?;
        // Workspace-only notes and a second Vault must stay out of this browsing range.
        let local = db.add(None, "本機", "#拍攝/地方")?;
        let other = db.import_vault("其他批次", vec![crate::archive::ImportedPage {
            parts: vec!["鏡頭.md".into()], title: "鏡頭".into(), body: "#拍攝/借景".into()
        }])?;
        let tree = db.vault_tag_tree(root)?;
        let shooting = &tree.branches[tree.branches[0].children["拍攝"]];
        assert_eq!(shooting.note_count, 2);
        assert!(shooting.exact);
        let night = &tree.branches[shooting.children["夜景"]];
        assert_eq!(night.note_count, 1);
        assert!(night.exact);
        assert_eq!(night.children.len(), 1);
        let numeric = &tree.branches[tree.branches[0].children["2026"]];
        assert!(!numeric.exact);
        assert_eq!(db.search_vault_sources(root, "", "", "2026", TagMatch::Descendants)?.len(), 1);
        assert!(db.search_vault_sources(root, "", "", "2026", TagMatch::Exact).is_err());
        let edit = &tree.branches[tree.branches[0].children["剪輯"]];
        assert_eq!(tree.branches[edit.children["ready"]].name, "READY");

        let exact = db.search_vault_sources(root, "", "", "#拍攝", TagMatch::Exact)?;
        assert_eq!(exact.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>(), ["外景/鏡頭.md"]);
        let hits = db.search_vault_sources(root, "", "", "#拍攝", TagMatch::Descendants)?;
        assert_eq!(hits.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>(), ["主棚/鏡頭.md", "外景/鏡頭.md"]);
        let id = hits[0].0;
        assert_eq!(
            db.search_vault_sources(root, "主棚", "主角", "拍攝", TagMatch::Descendants)?,
            [hits[0].clone()]
        );
        assert_eq!(db.quick_notes("tag:=#拍攝")?[0].id, exact[0].0);
        let shortcut = db.quick_notes("tag:#拍攝")?;
        assert_eq!(shortcut.len(), 3);
        assert!(shortcut.iter().any(|hit| hit.id == id));
        assert!(!shortcut.iter().any(|hit| hit.id == local));
        assert!(db.quick_notes("tag:##拍攝").is_err());
        db.trash(other)?;
        db.save(id, db.note(id)?.unwrap().version, "鏡頭", "已重新分類 #研究/日景")?;
        let backup = dir.join("archive.sqlite3");
        db.backup(&backup)?;
        drop(db);

        for database in [path, backup] {
            let db = Notebook::open(&database, false)?;
            let tree = db.vault_tag_tree(root)?;
            let shooting = &tree.branches[tree.branches[0].children["拍攝"]];
            assert_eq!(shooting.note_count, 1);
            assert!(shooting.children.is_empty());
            assert!(tree.branches[0].children.contains_key("研究"));
            let hits = db.search_vault_sources(root, "", "", "研究", TagMatch::Descendants)?;
            assert_eq!(hits, [(id, "主棚/鏡頭.md".to_string())]);
            assert_eq!(db.quick_notes("tag:#拍攝")?.len(), 1);
        }
        for (path, body) in originals {
            assert_eq!(std::fs::read(vault.join(path))?, body.as_bytes());
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }


    #[test]
    fn tag_panel_orders_counts_and_intersects_the_same_source_after_offline_reopen() -> Result<()> {
        use std::fs;
        let dir = super::super::tests::data_dir();
        let source = dir.join("Vault");
        let detached = dir.join("離線來源");
        let database = dir.join("work.sqlite");
        let backup = dir.join("old.sqlite");
        let files = [
            ("甲/鏡頭.md", "#Alpha #alpha #Alpha/夜景"),
            ("乙/鏡頭.md", "#alpha/夜景/待剪 #beta"),
            ("丙/鏡頭.md", "#beta #alpha-other"),
            ("丁/鏡頭.md", "#beta #推論/清單"),
        ];
        for (relative, body) in files {
            let path = source.join(relative);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, body)?;
        }
        let mut db = Notebook::open(&database, false)?;
        let root = db.import_vault("來源", crate::vault::read(&source)?.pages)?;
        db.import_vault("另一批", crate::vault::read(&source)?.pages)?;
        db.backup(&backup)?;
        drop(db);
        fs::rename(&source, &detached)?;

        for database in [database, backup] {
            let db = Notebook::open(&database, false)?;
            let tree = db.vault_tag_tree(root)?;
            let lookup = |name: &str| tree.branches[0].children[name];
            let (alpha, beta, inferred) = (lookup("alpha"), lookup("beta"), lookup("推論"));
            let child = tree.branches[alpha].children["夜景"];
            let grandchild = tree.branches[child].children["待剪"];
            assert_eq!((tree.branches[alpha].note_count, tree.branches[alpha].exact_count), (2, 1));
            assert_eq!((tree.branches[child].note_count, tree.branches[child].exact_count), (2, 1));
            assert_eq!(tree.path(grandchild), "alpha/夜景/待剪");
            let mut panel = TagPanel::default();
            assert_eq!(panel.rows(&tree).first(), Some(&(alpha, 0)));
            assert!(!panel.rows(&tree).contains(&(child, 1)));
            panel.frequency = true;
            assert_eq!(panel.rows(&tree).first(), Some(&(beta, 0)));
            panel.set_all(true);
            assert!(panel.rows(&tree).contains(&(grandchild, 2)));
            panel.toggle_branch(alpha);
            assert!(!panel.rows(&tree).contains(&(child, 1)));
            panel.toggle_branch(alpha);
            assert!(panel.rows(&tree).contains(&(grandchild, 2)));
            panel.set_all(false);
            assert!(!panel.rows(&tree).contains(&(grandchild, 2)));

            panel.toggle_term(alpha)?;
            panel.flat = true;
            assert_eq!(panel.rows(&tree).first(), Some(&(beta, 0)));
            assert!(!panel.rows(&tree).iter().any(|&(id, _)| id == inferred));
            assert!(panel.rows(&tree).contains(&(grandchild, 0)));
            panel.toggle_term(beta)?;
            assert_eq!(panel.terms(&tree), [("alpha".to_owned(), TagMatch::Descendants), ("beta".to_owned(), TagMatch::Exact)]);
            let hits = db.search_vault_tag_terms(root, &panel.terms(&tree))?;
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].1, "乙/鏡頭.md");
            assert!(db.vault_search_hit(root, hits[0].0, "乙/鏡頭.md")?);
            assert_eq!(db.note(hits[0].0)?.unwrap().body, "#alpha/夜景/待剪 #beta");
            panel.toggle_term(alpha)?;
            panel.toggle_term(alpha)?;
            assert!(db.search_vault_tag_terms(root, &panel.terms(&tree))?.is_empty());
            panel.clear_terms();
            assert_eq!(panel.term_count(), 0);
        }
        for (relative, body) in files {
            assert_eq!(fs::read(detached.join(relative))?, body.as_bytes());
        }
        assert!(!source.exists());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn vault_folder_browser_uses_saved_paths_after_reopen_and_source_disconnect() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let source = dir.join("原始 Vault");
        let detached = dir.join("拔除來源");
        let database = dir.join("workspace.sqlite");
        let originals = [
            ("片場/鏡頭.md", "---\naliases: [主棚, \"燈光, 鏡位\", 主棚]\n---\n甲棚標記"),
            ("片場/細節/檢查.md", "巢狀資料夾"),
            ("片場二/鏡頭.md", "---\naliases:\n- 外景\n---\n乙棚標記"),
            ("索引.md", "根目錄筆記"),
        ];
        for (relative, text) in originals {
            let path = source.join(relative);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, text)?;
        }

        let mut db = Notebook::open(&database, false)?;
        let root = db.import_vault("首批", crate::vault::read(&source)?.pages)?;
        let second = db.import_vault("同名來源的其他批次", crate::vault::read(&source)?.pages)?;
        let by_path = |db: &Notebook, root, folder: &str, path: &str| -> Result<i64> {
            db.vault_source_children(root, folder)?.into_iter().find_map(|item| match item {
                VaultSourceItem::Note { id, path: found } if found == path => Some(id),
                _ => None,
            }).ok_or("缺少來源筆記".into())
        };

        assert_eq!(
            &db.vault_source_children(root, "")?[..2],
            &[VaultSourceItem::Folder("片場".into()), VaultSourceItem::Folder("片場二".into())],
        );
        assert!(db.vault_source_children(root, "片")?.is_empty());
        assert!(by_path(&db, root, "", "索引.md").is_ok());
        let a = by_path(&db, root, "片場", "片場/鏡頭.md")?;
        let b = by_path(&db, root, "片場二", "片場二/鏡頭.md")?;
        assert_ne!(a, b);
        assert_ne!(a, by_path(&db, second, "片場", "片場/鏡頭.md")?);
        assert!(db.vault_source_children(root, "片場")?.contains(&VaultSourceItem::Folder("片場/細節".into())));
        assert!(by_path(&db, root, "片場/細節", "片場/細節/檢查.md").is_ok());
        let note = db.note(a)?.unwrap();
        assert_eq!(vault_aliases_preview(&note.body)?, ["主棚", "燈光, 鏡位"]);
        db.relocate(a, None)?;
        db.save(a, note.version, "工作區新標題", &note.body)?;
        db.trash(b)?;
        drop(db);
        std::fs::rename(&source, &detached)?;

        let mut db = Notebook::open(&database, false)?;
        assert_eq!(by_path(&db, root, "片場", "片場/鏡頭.md")?, a);
        assert_eq!(db.note(a)?.unwrap().row.title, "工作區新標題");
        assert!(db.vault_source_children(root, "片場二")?.is_empty());
        assert!(!db.vault_search_hit(root, b, "片場二/鏡頭.md")?);
        db.restore(b)?;
        assert_eq!(by_path(&db, root, "片場二", "片場二/鏡頭.md")?, b);
        assert!(db.note(b)?.unwrap().body.contains("乙棚標記"));
        db.trash(root)?;
        assert!(db.vault_source_children(root, "")?.is_empty());
        assert!(!db.vault_search_hit(root, a, "片場/鏡頭.md")?);
        assert!(by_path(&db, second, "片場", "片場/鏡頭.md").is_ok());

        assert!(!source.exists());
        for (relative, text) in originals {
            assert_eq!(std::fs::read(detached.join(relative))?, text.as_bytes());
        }
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

}

#[cfg(test)]
mod exact_folder_scope_tests {
    use super::*;
    use std::{fs, io::Read};

    #[test]
    fn fullwidth_space_distinguishes_scoped_chinese_folders_after_reopen() -> Result<()> {
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
        }
        let scratch = Scratch(super::super::tests::data_dir());
        let vault = scratch.0.join("研究來源");
        let make = |title| format!("---\naliases: [構圖]\n---\n# 同名\n{title}");
        let active = [("研究", "主要"), ("取景", "同名異處")];
        let omitted = "　研究";
        let sources = [active[0], active[1], (omitted, "已略過")];
        for (folder, expected) in sources {
            fs::create_dir_all(vault.join(folder))?;
            fs::write(vault.join(folder).join("構圖.md"), make(expected))?;
        }
        let path = scratch.0.join("工作區.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let scanned = crate::vault::read(&vault)?;
        assert_eq!(scanned.incompatible, 1);
        let root = db.import_vault("研究", scanned.pages)?;
        let other = db.import_vault("研究", crate::vault::read(&vault)?.pages)?;
        drop(db);
        let db = Notebook::open(&path, false)?;
        assert!(db.search_vault_sources(root, omitted, "構圖", "", TagMatch::Exact)?.is_empty(),
            "omitted source folder must not redirect to a sibling");
        let blank = db.search_vault_sources(root, "", "構圖", "", TagMatch::Exact)?;
        assert_eq!(blank.len(), 2);
        assert_eq!(db.search_vault_sources(root, "　 ", "構圖", "", TagMatch::Exact)?, blank);
        for (folder, expected) in active {
            let hits = db.search_vault_sources(root, folder, "構圖", "", TagMatch::Exact)?;
            assert_eq!(hits.len(), 1);
            let destination = format!("{folder}/構圖.md");
            assert_eq!(hits[0].1, destination, "folder scope must be exact");
            assert_eq!(db.note(hits[0].0)?.unwrap().body, make(expected));
            assert!(!db.vault_search_hit(other, hits[0].0, &destination)?);
        }
        let path = scratch.0.join("匯出.zip");
        assert_eq!(db.export_vault_markdown(root, &path)?, 2);
        let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
        assert!(zip.by_name(&format!("{omitted}/構圖.md")).is_err());
        for (folder, expected) in active {
            let mut actual = Vec::new();
            zip.by_name(&format!("{folder}/構圖.md"))?.read_to_end(&mut actual)?;
            assert_eq!(actual, make(expected).as_bytes());
        }
        Ok(())
    }
}
