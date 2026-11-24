use super::super::{Notebook, Note, Result, Row, read_row};
use super::{NATIVE_NOTE_PREFIX, native_note_target};
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
        let indexes:HashMap<_,_>=vaults.iter().map(|(root,files)|(*root,crate::vault::WikiIndex::new(files))).collect();
        // A missing embed must not start showing a namesake merely because
        // one of the original candidates was moved into the recycle bin.
        let mut archived = HashMap::<i64, Vec<(i64, String)>>::new();
        if tx.prepare("SELECT 1 FROM notes n LEFT JOIN vault_paths p ON p.note_id=n.id
            WHERE n.trash_root IS NULL AND (?1 IS NULL OR p.import_root=?1)
            AND instr(n.body,'![[')>0")?.exists([root])? {
            let mut stmt = tx.prepare("SELECT p.import_root,p.note_id,p.relative_path FROM vault_paths p
                JOIN notes n ON n.id=p.note_id WHERE n.trash_root IS NOT NULL
                AND (?1 IS NULL OR p.import_root=?1)")?;
            let mut rows = stmt.query([root])?;
            while let Some(row) = rows.next()? {
                archived.entry(row.get(0)?).or_default().push((row.get(1)?, row.get(2)?));
            }
        }
        let embed_indexes: HashMap<_, _> = archived.iter().filter_map(|(root, archived)| {
            vaults.get(root).map(|active| (*root, crate::vault::WikiIndex::including(active, archived)))
        }).collect();
        let mut edges=BTreeSet::new();
        let mut consumed=0usize;
        {
            let mut stmt=tx.prepare("SELECT n.id,n.body FROM notes n LEFT JOIN vault_paths p ON n.id=p.note_id
                WHERE n.trash_root IS NULL AND (?1 IS NULL OR p.import_root=?1)
                AND instr(n.body,'[[')>0 ORDER BY n.id")?;
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
                let keys = crate::markdown::wiki_targets(body);
                let location = &nodes[&id].location;
                let mut bound = HashMap::<String,i64>::new();
                if !keys.is_empty() && location.is_none() {
                    let records = bindings.query_map([id], |row| Ok((row.get(0)?,row.get(1)?)))?;
                    bound = records.collect::<rusqlite::Result<_>>()?;
                }
                let direct = keys.into_iter().map(|key| {
                    if key.starts_with(NATIVE_NOTE_PREFIX) {
                        let path = key.split('#').next().unwrap_or(&key);
                        return native_note_target(&tx, path);
                    }
                    Ok(if let Some((root,path))=location {
                        indexes[root].resolve(path,&key)
                    } else {
                        key.strip_prefix('#').and_then(|s|s.parse().ok())
                            .or_else(|| bound.get(&key).copied())
                            .or_else(||titles.get(&key).copied().flatten())
                    })
                });
                let vault = location.as_ref().map(|(root, path)|
                    (path.as_str(), embed_indexes.get(root).unwrap_or(&indexes[root])));
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
        let make=|folder:&str,title:&str,body:&str| ImportedPage {
            title:title.into(),body:body.into(),parts:vec![folder.into(),format!("{title}.md")],
        };
        let root=db.import_vault("圖測試",vec![
            make("A","入口","[[../B/同名#段落|前往]] [[同名]] `[[./同名]]`"),
            make("A","同名",""), make("B","同名",&format!("[[paperroom:note:{outside}|外部]]")),
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
}
