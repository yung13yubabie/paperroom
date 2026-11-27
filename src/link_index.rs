use super::super::{Notebook, Note, Result, Row, read_row};
use super::{NATIVE_NOTE_PREFIX, is_heptabase_root, mapped_link_target, native_note_target};
use crate::markdown::{self, AnchorSyntax};
use crate::store::transclusion::{embed_references, resolve_embed_target};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

const MAX_GRAPH_NODES: usize = 5_000;
const MAX_GRAPH_EDGES: usize = 20_000;
const MAX_GRAPH_TEXT: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope { Workspace, Vault(i64) }

pub(crate) struct IndexNode {
    pub row: Row,
    pub location: Option<(i64, String)>,
}

impl IndexNode {
    pub fn detail(&self) -> String {
        self.location.as_ref().map_or_else(|| format!("#{}", self.row.id),
            |(_, path)| format!("{path}  ·  #{}", self.row.id))
    }
}

pub(crate) struct LinkIndex {
    pub nodes: BTreeMap<i64, IndexNode>,
    pub edges: BTreeSet<(i64, i64)>,
}

impl LinkIndex {
    pub fn shortest_path(&self, start: i64, end: i64) -> Option<Vec<i64>> {
        if !self.nodes.contains_key(&start) || !self.nodes.contains_key(&end) { return None; }
        let mut queue = VecDeque::from([start]);
        let mut previous = HashMap::from([(start, start)]);
        while let Some(current) = queue.pop_front() {
            if current == end {
                let mut route = vec![end];
                let mut cursor = end;
                while cursor != start { cursor = previous[&cursor]; route.push(cursor); }
                route.reverse();
                return Some(route);
            }
            for &(_, target) in self.edges.range((current,i64::MIN)..=(current,i64::MAX)) {
                if let std::collections::hash_map::Entry::Vacant(slot) = previous.entry(target) {
                    slot.insert(current);
                    queue.push_back(target);
                }
            }
        }
        None
    }
}

impl Notebook {
    pub(crate) fn link_index(&self, scope: Scope, focus: Option<&Note>) -> Result<LinkIndex> {
        // Both streams see the same SQLite snapshot, even if another window saves mid-scan.
        let tx = self.connection.unchecked_transaction()?;
        let root = match scope { Scope::Workspace=>None, Scope::Vault(root)=>Some(root) };
        let mut nodes = BTreeMap::new();
        let mut titles = HashMap::<String,Option<i64>>::new();
        let mut vaults = HashMap::<i64, Vec<(i64,String)>>::new();
        {
            let mut stmt=tx.prepare("SELECT n.id,n.parent,n.title,n.pinned,n.trash_root,p.import_root,p.relative_path
                FROM notes n LEFT JOIN vault_paths p ON n.id=p.note_id
                WHERE n.trash_root IS NULL AND (?1 IS NULL OR p.import_root=?1) ORDER BY n.id")?;
            let mut rows=stmt.query([root])?;
            while let Some(record)=rows.next()? {
                if focus.is_none() && nodes.len()>=MAX_GRAPH_NODES {
                    return Err("此範圍超過 5,000 篇筆記，請改用目前筆記或較小 Vault 的關聯圖。".into());
                }
                let row=read_row(record)?;
                titles.entry(row.title.clone()).and_modify(|found| *found=None).or_insert(Some(row.id));
                let location=match record.get::<_,Option<i64>>(5)? {
                    Some(root)=>{
                        let path:String=record.get(6)?;
                        vaults.entry(root).or_default().push((row.id,path.clone()));
                        Some((root,path))
                    }
                    None=>None,
                };
                nodes.insert(row.id,IndexNode {row,location});
            }
        }
        // Include trash in disambiguation, then exclude those IDs from edges.
        // Removing one of two namesakes must never repoint a link to the other.
        if !vaults.is_empty() {
            let mut stmt = tx.prepare("SELECT p.import_root,p.note_id,p.relative_path FROM vault_paths p
                JOIN notes n ON n.id=p.note_id WHERE n.trash_root IS NOT NULL
                AND (?1 IS NULL OR p.import_root=?1)")?;
            let mut rows = stmt.query([root])?;
            while let Some(row) = rows.next()? {
                if let Some(paths) = vaults.get_mut(&row.get::<_, i64>(0)?) {
                    paths.push((row.get(1)?, row.get(2)?));
                }
            }
        }
        let indexes = vaults.iter().map(|(&root, files)| {
            Ok((root, (crate::vault::NoteLinkPaths::new(files), is_heptabase_root(&tx, root)?)))
        }).collect::<Result<HashMap<_, _>>>()?;
        let mut edges=BTreeSet::new();
        let mut consumed=0usize;
        {
            let mut stmt=tx.prepare("SELECT n.id,n.body FROM notes n LEFT JOIN vault_paths p ON n.id=p.note_id
                WHERE n.trash_root IS NULL AND (?1 IS NULL OR p.import_root=?1)
                AND instr(n.body,'[')>0 ORDER BY n.id")?;
            let mut bindings = tx.prepare(
                "SELECT destination,target_id FROM note_title_link_bindings WHERE source_id=?1",
            )?;
            let mut rows=stmt.query([root])?;
            while let Some(record)=rows.next()? {
                let id=record.get(0)?;
                // Use the saved body from this same snapshot, including when
                // the window still has an earlier revision open.
                let body=record.get_ref(1)?.as_str()?;
                consumed=consumed.saturating_add(body.len());
                if focus.is_none() && consumed>MAX_GRAPH_TEXT {
                    return Err("需解析的含連結正文超過 64 MiB，請縮小關聯圖範圍。".into());
                }
                let references = markdown::note_references(body);
                let location = &nodes[&id].location;
                let mut bound = HashMap::<String,i64>::new();
                if location.is_none() && references.iter().any(|link| link.syntax == AnchorSyntax::Wiki) {
                    let records = bindings.query_map([id], |row| Ok((row.get(0)?,row.get(1)?)))?;
                    bound = records.collect::<rusqlite::Result<_>>()?;
                }
                let parts = location.as_ref().map(|(_, path)| path.split('/').map(str::to_owned).collect::<Vec<_>>());
                let direct = references.into_iter().map(|reference| {
                    let key = reference.destination.as_str();
                    let path = key.split_once('#').map_or(key, |(path, _)| path);
                    if path.starts_with(NATIVE_NOTE_PREFIX) {
                        return native_note_target(&tx, Some(id), path);
                    }
                    Ok(if let Some((root, source)) = location {
                        let (index, exact_only) = &indexes[root];
                        mapped_link_target(index, *exact_only, source, parts.as_deref().unwrap_or_default(),
                            reference.syntax, key, path)
                    } else if reference.syntax == AnchorSyntax::Wiki {
                        key.strip_prefix('#').and_then(|s|s.parse().ok())
                            .or_else(|| bound.get(key).copied())
                            .or_else(||titles.get(key).copied().flatten())
                    } else {
                        None
                    })
                });
                let vault = location.as_ref().and_then(|(root, path)| {
                    let (index, exact_only) = &indexes[root];
                    (!exact_only).then_some((path.as_str(), &index.wiki))
                });
                let embeds = embed_references(body).map(|reference|
                    resolve_embed_target(&tx, id, vault, &reference.destination));
                for target in direct.chain(embeds) {
                    if let Some(target) = target?.filter(|target| nodes.contains_key(target))
                        && focus.is_none_or(|n|n.row.id==id || n.row.id==target) {
                        edges.insert((id,target));
                        if focus.is_none() && edges.len()>MAX_GRAPH_EDGES {
                            return Err("此範圍超過 20,000 組有向連結，請縮小關聯圖範圍。".into());
                        }
                    }
                }
            }
        }
        tx.commit()?;
        Ok(LinkIndex {nodes,edges})
    }

    pub(crate) fn indexed_related(&self,note:&Note)->Result<(Vec<Row>,Vec<Row>)> {
        let index=self.link_index(Scope::Workspace,Some(note))?;
        let mut outgoing=Vec::new();
        let mut incoming=Vec::new();
        for (source,target) in index.edges {
            if source==note.row.id {outgoing.push(index.nodes[&target].row.clone());}
            if target==note.row.id && source!=target {incoming.push(index.nodes[&source].row.clone());}
        }
        Ok((outgoing,incoming))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use std::path::Path;

    #[test]
    fn workspace_path_is_directed_vault_isolated_and_snapshot_refreshes() -> Result<()> {
        let mut db=Notebook::open(Path::new(":memory:"),false)?;
        let outside=db.add(None,"同名","")?;
        let token: String = db.connection.query_row(
            "SELECT token FROM note_link_identities WHERE note_id=?1", [outside], |row| row.get(0),
        )?;
        let make=|folder:&str,title:&str,body:&str| ImportedPage {
            title:title.into(),body:body.into(),parts:vec![folder.into(),format!("{title}.md")],
        };
        let root=db.import_vault("圖測試",vec![
            make("A","入口","[[../B/同名#段落|前往]] [[同名]] `[[./同名]]`"),
            make("A","同名",""), make("B","同名",&crate::store::note_links::note_link_markup(outside, &token, "外部")),
        ])?;
        let local=db.link_index(Scope::Vault(root),None)?;
        let id=|path:&str|local.nodes.values().find(|node|
            node.location.as_ref().is_some_and(|(_,p)|p==path)).unwrap().row.id;
        let start=id("A/入口.md"); let goal=id("B/同名.md"); let duplicate=id("A/同名.md");
        assert_eq!(local.edges, BTreeSet::from([(start,goal)]));
        assert!(!local.nodes.contains_key(&outside));
        let workspace=db.link_index(Scope::Workspace,None)?;
        assert_eq!(workspace.shortest_path(start,outside),Some(vec![start,goal,outside]));
        assert_eq!(workspace.shortest_path(outside,start),None);
        assert_eq!(workspace.shortest_path(start,duplicate),None);
        assert_eq!(workspace.shortest_path(duplicate,duplicate),Some(vec![duplicate]));
        db.trash(goal)?;
        assert!(db.link_index(Scope::Workspace,None)?.shortest_path(start,outside).is_none());
        assert_eq!(workspace.shortest_path(start,outside),Some(vec![start,goal,outside]));
        Ok(())
    }

    #[test]
    fn scoped_overview_rejects_oversize_instead_of_showing_a_partial_path() -> Result<()> {
        let db=Notebook::open(Path::new(":memory:"),false)?;
        db.connection.execute("WITH RECURSIVE seq(n) AS (
            VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<?1
        ) INSERT INTO notes(title) SELECT printf('合成頁%d',n) FROM seq", [MAX_GRAPH_NODES as i64+1])?;
        assert!(db.link_index(Scope::Workspace,None).err().unwrap().to_string().contains("5,000"));
        Ok(())
    }

    #[test]
    fn heptabase_menu_graph_audit_and_embeds_share_exact_source_policy() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let page = |name: &str, body: &str| ImportedPage {
            title: name.rsplit('/').next().unwrap().into(),
            parts: name.split('/').map(str::to_owned).collect(), body: body.into(),
        };
        let body = "[正確](../乙/同名.md#^take) [省略副檔](../乙/同名#^take) [缺少同層](根稿.md#^only) [[根稿#^only|舊式名稱]]

![[根稿]]";
        let root = db.import_vault("合成來源", vec![
            page("甲/本文.md", body), page("乙/同名.md", "選區 ^take"),
            page("甲/同名.md", "同名另一篇"), page("根稿.md", "僅在根目錄的內容 ^only"),
        ])?;
        let id = |path: &str| db.connection.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            (root, path), |row| row.get::<_, i64>(0));
        let host = id("甲/本文.md")?;
        let target = id("乙/同名.md")?;
        let other = id("根稿.md")?;
        let source = db.note(host)?.unwrap();
        assert!(db.vault_note_links(&source)?.iter().any(|link| link.note_id == other));
        assert!(db.project_embeds(host, body)?.contains("僅在根目錄的內容"));
        db.connection.execute("INSERT INTO heptabase_import_roots(import_root) VALUES(?1)", [root])?;
        let links = db.vault_note_links(&source)?;
        assert_eq!(links.iter().map(|link| (link.note_id, link.label.as_str())).collect::<Vec<_>>(), [(target,"正確")]);
        assert_eq!(db.related(&source)?.0.iter().map(|row| row.id).collect::<Vec<_>>(), [target]);
        assert!(db.vault_incoming_block_report(target)?.contains("有 1 個明確指向"));
        assert!(db.vault_incoming_block_report(other)?.contains("有 0 個明確指向"));
        assert!(!db.project_embeds(host, body)?.contains("僅在根目錄的內容"));
        Ok(())
    }

    #[test]
    fn markdown_edges_follow_original_vault_paths_and_ambiguous_paths_do_not_retarget() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let body = "[直接](../乙/同名.md) [標題](../乙/同名.md#節) [歧義](同名.md) [[同名|歧義]] `[程式碼](../甲/程式.md)` [外部](https://example.invalid/同名.md)";
        let page = |file: &str, title: &str, text: &str| ImportedPage {
            parts: file.split('/').map(str::to_owned).collect(),
            title: title.into(), body: text.into(),
        };
        let root = db.import_vault("Vault", vec![
            page("甲/源.md", "源", body), page("同名.md", "同名", "外層"),
            page("甲/同名.md", "同名", "同層"), page("乙/同名.md", "同名", "# 節"),
            page("甲/程式.md", "程式", ""),
        ])?;
        let find = |file| db.connection.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            (root, file), |r| r.get::<_, i64>(0),
        );
        let source = find("甲/源.md")?;
        let target = find("乙/同名.md")?;
        let collision = find("同名.md")?;
        let source_note = db.note(source)?.unwrap();
        assert_eq!(db.related(&source_note)?.0.iter().map(|r| r.id).collect::<Vec<_>>(), [target]);
        let refs = db.vault_note_links(&source_note)?;
        assert_eq!(refs.iter().map(|r| (r.note_id, r.anchor.as_deref())).collect::<Vec<_>>(),
            [(target, None), (target, Some("#節"))]);
        db.trash(collision)?;
        assert_eq!(db.related(&source_note)?.0.iter().map(|r| r.id).collect::<Vec<_>>(), [target]);
        assert_eq!(db.vault_note_links(&source_note)?.len(), 2);
        db.save(target, 1, "已改標題", "# 節")?;
        drop(db);

        let db = Notebook::open(&path, false)?;
        let target_note = db.note(target)?.unwrap();
        assert_eq!(target_note.row.title, "已改標題");
        assert_eq!(db.related(&target_note)?.1[0].id, source);
        assert_eq!(db.link_index(Scope::Vault(root), None)?.shortest_path(source, target), Some(vec![source, target]));
        assert_eq!(db.note(source)?.unwrap().body, body);
        db.trash(target)?;
        assert!(db.vault_note_links(&source_note)?.is_empty());
        assert!(db.related(&source_note)?.0.is_empty());
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
