//! Update only proven heading and link source spans inside one imported Vault snapshot.
use super::{AnchorSyntax, Notebook, NoteLinkPaths, Result, is_heptabase_root, mapped_link_target, native_note_target, NATIVE_NOTE_PREFIX};
use crate::markdown::{self, HeadingLookup};
use rusqlite::{Connection,OptionalExtension,params,TransactionBehavior};
use std::ops::Range;

pub(crate) struct HeadingSelection {
    target:i64, version:i64, token:String, pub titles:Vec<String>,
}

#[derive(PartialEq,Eq)]
struct Edit { id:i64, version:i64, token:String, title:String, path:String, next:String, links:usize }

#[derive(PartialEq,Eq)]
pub(crate) struct RenamePlan {
    root:i64, target:i64, old:String, new:String, edits:Vec<Edit>,
    canvases:Vec<super::super::canvas_links::CanvasHeadingEdit>,
}

impl RenamePlan {
    pub(crate) fn summary(&self)->String {
        let links=self.edits.iter().map(|e|e.links).sum::<usize>();
        let mut out=format!("「{}」 → 「{}」\r\n將更新 {} 篇筆記、{} 個連結。只修改工作區內同批 Vault 的使用中筆記。\r\n",self.old,self.new,self.edits.len(),links);
        for e in &self.edits {out.push_str(&format!("\r\n{}　[#{}]　{} 個連結",e.path,e.id,e.links));}
        for canvas in &self.canvases { for node in &canvas.nodes {
            out.push_str(&format!("\r\nCanvas {}　[#{}]　節點：{}",canvas.title,canvas.id,node));
        }}
        out
    }
}

fn plain(s:&str)->bool {
    !s.is_empty() && s.len()<=256 && s.trim()==s &&
        s.chars().all(|c|c.is_alphanumeric() || matches!(c,' '|'-'|'_'))
}

fn heading_span(body:&str, pos:usize, level:u8, title:&str)->Option<Range<usize>> {
    let line=body.get(pos..)?.split(['\n','\r']).next()?;
    let marker="#".repeat(level.into());
    let tail=line.strip_prefix(&marker)?;
    if !tail.starts_with([' ','\t']) {return None;}
    let rest=tail.trim_start_matches([' ','\t']);
    if rest.trim_end_matches([' ','\t'])!=title || !plain(title) {return None;}
    let at=pos+line.len()-rest.len();
    Some(at..at+title.len())
}

fn fragment_span(body:&str, link:&markdown::NoteReference)->Option<Range<usize>> {
    let raw=body.get(link.offset..link.end)?;
    let dest=&link.destination;
    let anchor=dest.find('#')?;
    let at=match link.syntax {
        AnchorSyntax::Wiki=>{
            let inner=raw.strip_prefix("[[")?.strip_suffix("]]")?;
            if inner.split('|').next()?!=dest {return None;}
            2
        }
        AnchorSyntax::Markdown=>{
            let inner=raw.strip_suffix(')')?;
            let before=inner.strip_suffix(dest)?;
            if !before.ends_with("]("){return None;}
            before.len()
        }
        _=>return None,
    };
    Some(link.offset+at+anchor..link.offset+at+dest.len())
}

fn encoded(s:&str)->String {
    use std::fmt::Write;
    let mut out=String::from("#");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b,b'-'|b'_'|b'.'|b'~') {out.push(b as char);}
        else {write!(out,"%{b:02X}").expect("String write");}
    }
    out
}

impl Notebook {
    pub(crate) fn vault_heading_choices(&self,id:i64)->Result<HeadingSelection> {
        let (root,body,version,token):(i64,String,i64,String)=self.connection.query_row(
            "SELECT p.import_root,n.body,n.version,i.token FROM vault_paths p JOIN notes n ON n.id=p.note_id
             JOIN note_link_identities i ON i.note_id=n.id JOIN notes r ON r.id=p.import_root WHERE n.id=? AND n.trash_root IS NULL AND r.trash_root IS NULL",
            [id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or("請選擇已匯入 Vault 的筆記。")?;
        if is_heptabase_root(&self.connection,root)? {return Err("此功能適用於匯入的 Obsidian Vault。".into());}
        let heads=markdown::source_headings(&body);
        if heads.len()>2000 {return Err("小節數量超過 2000。".into());}
        let mut counts=std::collections::HashMap::new();
        for (_,_,title) in &heads {*counts.entry(title.as_str()).or_insert(0usize)+=1;}
        let titles=heads.iter().filter(|(at,level,title)| counts[title.as_str()]==1 &&
            heading_span(&body,*at,*level,title).is_some()).map(|(_,_,title)|title.clone()).collect();
        Ok(HeadingSelection{target:id,version,token,titles})
    }

    pub(crate) fn preview_heading_rename(&self,selection:&HeadingSelection,old:&str,new:&str)->Result<RenamePlan> {
        let tx=self.connection.unchecked_transaction()?;
        let plan=prepare(&tx,selection.target,selection.version,old,new)?;
        if !plan.edits.iter().any(|edit| edit.id==selection.target && edit.token==selection.token) {
            return Err("筆記身分已變更，請重新開啟選單。".into());
        }
        tx.commit()?;Ok(plan)
    }

    pub(crate) fn apply_heading_rename(&self,plan:&RenamePlan)->Result<()> {
        let tx=rusqlite::Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        let version=plan.edits.iter().find(|e|e.id==plan.target).ok_or("缺少小節來源。")?.version;
        if prepare(&tx,plan.target,version,&plan.old,&plan.new)?!=*plan {
            return Err("預覽後來源或引用已變更，請重新檢查。".into());
        }
        for e in &plan.edits { super::super::save_in(&tx,e.id,e.version,&e.title,&e.next)?; }
        for c in &plan.canvases {
            super::super::canvas_edit::replace_with_undo(&tx,c.id,&c.original,&c.revised)?;
        }
        tx.commit()?;Ok(())
    }
}

fn prepare(db:&Connection,target:i64,version:i64,old:&str,new:&str)->Result<RenamePlan> {
    if !plain(new) || old==new {return Err("新名稱應與原名稱不同，並只使用中英文字、數字、空格、底線或連字號。".into());}
    let (root,body):(i64,String)=db.query_row("SELECT p.import_root,n.body FROM vault_paths p JOIN notes n ON n.id=p.note_id
        JOIN notes r ON r.id=p.import_root WHERE n.id=? AND n.version=? AND n.trash_root IS NULL AND r.trash_root IS NULL",
        params![target,version],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or("目標筆記已變更，請重新開啟。")?;
    if is_heptabase_root(db,root)? {return Err("此功能適用於匯入的 Obsidian Vault。".into());}
    let before=markdown::source_headings(&body);
    if before.len()>2000 {return Err("小節數量超過 2000。".into());}
    let mut matching=before.iter().enumerate().filter(|(_,(_,_,title))|title==old);
    let (ordinal,(pos,level,title))=matching.next().ok_or("找不到原小節。")?;
    if matching.next().is_some() || before.iter().any(|(_,_,t)|t==new) {return Err("小節名稱不唯一，請改用其他名稱。".into());}
    let heading=heading_span(&body,*pos,*level,title).ok_or("只支援單行純文字 ATX 小節。")?;
    let mut changed=body.clone();
    changed.replace_range(heading.clone(),new);
    let after=markdown::source_headings(&changed);
    if after.len()!=before.len() || after[ordinal].2!=new {return Err("新名稱無法作為純文字標題。".into());}
    let (count,bytes):(i64,i64)=db.query_row("SELECT count(*),coalesce(sum(length(CAST(n.body AS BLOB))),0)
        FROM vault_paths p JOIN notes n ON n.id=p.note_id WHERE p.import_root=?", [root],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if count>5000 || bytes>16*1024*1024 {return Err("同批筆記超過此次檢查上限（5000 篇或 16 MiB）。".into());}
    let mut q=db.prepare("SELECT note_id,relative_path FROM vault_paths WHERE import_root=? ORDER BY note_id")?;
    let paths=q.query_map([root],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let index=NoteLinkPaths::new(&paths);
    let mut q=db.prepare("SELECT n.id,n.title,n.body,n.version,i.token,p.relative_path FROM vault_paths p JOIN notes n ON n.id=p.note_id
        JOIN note_link_identities i ON i.note_id=n.id WHERE p.import_root=? AND n.trash_root IS NULL ORDER BY n.id")?;
    let mut rows=q.query([root])?;
    let mut edits=Vec::new();
    let mut comparisons=0usize;
    while let Some(row)=rows.next()? {
        let id:i64=row.get(0)?;
        let source:String=row.get(2)?;
        let path:String=row.get(5)?;
        let parts=path.split('/').map(str::to_owned).collect::<Vec<_>>();
        let mut spans=Vec::new();
        let (_,prose)=super::super::vault_search::frontmatter(&source);
        let skip=source.len()-prose.len();
        for link in markdown::note_references(prose) {
            let Some((dest,frag))=link.destination.split_once('#') else {continue};
            let resolved=if dest.is_empty(){Some(id)}else if dest.starts_with(NATIVE_NOTE_PREFIX){
                native_note_target(db,Some(id),dest)?
            }else{mapped_link_target(&index,false,&path,&parts,link.syntax,&link.destination,dest)};
            if resolved!=Some(target){continue;}
            comparisons+=before.len();
            if comparisons>2_000_000 {return Err("引用比對超過此次檢查上限。".into());}
            let anchor=format!("#{frag}");
            let lookup=|heads:&[_],len,a:&str| match markdown::lookup_headings(heads,len,a,link.syntax) {
                HeadingLookup::Found{ordinal,..}=>Some(ordinal),_=>None,
            };
            let was=lookup(&before,body.len(),&anchor);
            let now=lookup(&after,changed.len(),&anchor);
            if was==now {continue;}
            if was!=Some(ordinal) || frag.contains('#') {
                return Err(format!("{} 有階層引用或名稱衝突，無法安全保留原定位。",path).into());
            }
            let replacement=if link.syntax==AnchorSyntax::Wiki {format!("#{new}")}else{encoded(new)};
            if lookup(&after,changed.len(),&replacement)!=was {return Err("新名稱會產生歧義。".into());}
            let r=fragment_span(prose,&link).ok_or_else(||format!("{} 有無法安全修改的連結寫法。",path))?;
            spans.push((skip+r.start..skip+r.end,replacement));
        }
        let links=spans.len();
        if id==target {spans.push((heading.clone(),new.to_owned()));}
        if spans.is_empty(){continue;}
        if db.prepare_cached("SELECT 1 FROM note_highlights WHERE note_id=?")?.exists([id])? {
            return Err(format!("{} 有綁定正文位置的螢光標記，已停止以保留標記。",path).into());
        }
        spans.sort_by_key(|(r,_)|r.start);
        if spans.windows(2).any(|w|w[0].0.end>w[1].0.start){return Err("來源修改範圍重疊。".into());}
        let mut next=source;
        for (range,text) in spans.into_iter().rev(){next.replace_range(range,&text);}
        edits.push(Edit{id,version:row.get(3)?,token:row.get(4)?,title:row.get(1)?,path,next,links});
    }
    let canvases=super::super::canvas_links::plan_heading_nodes(db,root,target,|anchor|{
        comparisons+=before.len();
        if comparisons>2_000_000 {return Err("引用比對超過此次檢查上限。".into());}
        let locate=|heads:&[_],len,a:&str| markdown::lookup_headings(heads,len,a,AnchorSyntax::Wiki);
        let prior=locate(&before,body.len(),anchor);
        let next=locate(&after,changed.len(),anchor);
        let position=|v:&HeadingLookup|match v {HeadingLookup::Found{ordinal,..}=>Some(*ordinal),_=>None};
        if position(&prior)==position(&next) {return Ok(None);}
        let replacement=format!("#{new}");
        if position(&prior)!=Some(ordinal) || anchor[1..].contains('#')
            || position(&locate(&after,changed.len(),&replacement))!=Some(ordinal) {
            return Err("此小節會失去原定位或改指其他小節。".into());
        }
        Ok(Some(replacement))
    })?;
    Ok(RenamePlan{root,target,old:old.to_owned(),new:new.to_owned(),edits,canvases})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use std::fs;
    fn page(path:&str,body:&str)->ImportedPage {
        ImportedPage{parts:path.split('/').map(str::to_owned).collect(),title:"鏡頭".into(),body:body.into()}
    }
    #[test]
    fn ambiguous_and_unsupported_heading_changes_preserve_sources()->Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let root=db.import_vault("vault",vec![page("A.md","## Old\n## Taken\n"),page("B.md","[text][r]\n\n[r]: A.md#Old\n")])?;
        let id=|p:&str| -> Result<i64>{Ok(db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=? AND relative_path=?",
            params![root,p],|r|r.get(0))?)};
        let a=id("A.md")?;let b=id("B.md")?;
        let selected=db.vault_heading_choices(a)?;
        assert!(db.preview_heading_rename(&selected,"Old","Taken").is_err());
        assert!(db.preview_heading_rename(&selected,"Old","New").is_err());
        let s=db.note(b)?.unwrap();
        db.save(b,s.version,&s.row.title,"[[A#New]]")?; // would newly bind an existing missing anchor
        assert!(db.preview_heading_rename(&selected,"Old","New").is_err());
        let s=db.note(b)?.unwrap();
        db.save(b,s.version,&s.row.title,"[[A#Old]]")?;
        let before=db.note(b)?.unwrap().body;
        db.connection.execute("INSERT INTO note_highlights(note_id,start_byte,end_byte) VALUES(?,0,1)",[b])?;
        assert!(db.preview_heading_rename(&selected,"Old","New").is_err());
        assert_eq!(db.note(b)?.unwrap().body,before);
        assert_eq!(db.note(a)?.unwrap().body,"## Old\n## Taken\n");
        Ok(())
    }

    #[test]
    fn heading_rename_resolves_exact_vault_source_and_updates_atomically_across_reopen_backup()->Result<()> {
        let dir=super::super::super::tests::data_dir();
        let path=dir.join("workspace.sqlite3");
        let mut db=Notebook::open(&path,false)?;
        let body="---\r\nstatus: active\r\n---\r\n## 舊小節\r\n目標正文\n[本篇](#舊小節)\r\n";
        let refs="[[A/鏡頭#舊小節|原別名]] [路徑](../A/鏡頭.md#舊小節)\r\n[[B/鏡頭#舊小節]]\n`[[A/鏡頭#舊小節]]`\n";
        let root=db.import_vault("fixture",vec![page("A/鏡頭.md",body),page("B/鏡頭.md","## 舊小節\n另一篇"),page("B/入口.md",refs)])?;
        let note=|db:&Notebook,p:&str|->Result<i64>{
            Ok(db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=? AND relative_path=?",params![root,p],|r|r.get(0))?)
        };
        let target=note(&db,"A/鏡頭.md")?;let source=note(&db,"B/入口.md")?;
        let decoy=note(&db,"B/鏡頭.md")?;
        let selected=db.vault_heading_choices(target)?;
        assert_eq!(selected.titles,["舊小節"]);
        let plan=db.preview_heading_rename(&selected,"舊小節","新小節")?;
        assert_eq!(plan.edits.len(),2);
        assert_eq!(plan.edits.iter().map(|e|e.links).sum::<usize>(),3);
        assert_eq!(db.note(source)?.unwrap().body,refs);
        let s=db.note(source)?.unwrap();
        db.save(source,s.version,&s.row.title,&format!("{refs}新草稿"))?;
        assert!(db.apply_heading_rename(&plan).is_err());
        assert_eq!(db.note(target)?.unwrap().body,body);
        let plan=db.preview_heading_rename(&selected,"舊小節","新小節")?;
        db.connection.execute_batch(&format!("CREATE TEMP TRIGGER test_heading_failure BEFORE UPDATE OF body ON notes
            WHEN OLD.id={source} BEGIN SELECT RAISE(ABORT,'fixture failure'); END;"))?;
        assert!(db.apply_heading_rename(&plan).is_err());
        assert_eq!(db.note(target)?.unwrap().body,body);
        db.connection.execute_batch("DROP TRIGGER test_heading_failure;")?;
        db.apply_heading_rename(&plan)?;
        assert_eq!(db.note(target)?.unwrap().body,"---\r\nstatus: active\r\n---\r\n## 新小節\r\n目標正文\n[本篇](#%E6%96%B0%E5%B0%8F%E7%AF%80)\r\n");
        assert_eq!(db.note(source)?.unwrap().body,"[[A/鏡頭#新小節|原別名]] [路徑](../A/鏡頭.md#%E6%96%B0%E5%B0%8F%E7%AF%80)\r\n[[B/鏡頭#舊小節]]\n`[[A/鏡頭#舊小節]]`\n新草稿");
        assert_eq!(db.note(decoy)?.unwrap().body,"## 舊小節\n另一篇");
        drop(db);
        let db=Notebook::open(&path,false)?;
        let backup=dir.join("backup.sqlite3");db.backup(&backup)?;drop(db);
        let isolated=dir.join("restore.sqlite3");fs::copy(backup,&isolated)?;
        let db=Notebook::open(&isolated,false)?;
        let links=db.vault_note_links(&db.note(source)?.unwrap())?;
        assert_eq!(links.iter().map(|l|l.note_id).collect::<Vec<_>>(),[target,target,decoy]);
        for l in &links[..2] {
            assert!(matches!(markdown::resolve_heading(&db.note(target)?.unwrap().body,l.anchor.as_ref().unwrap(),l.anchor_syntax),HeadingLookup::Found{..}));
        }
        drop(db);fs::remove_dir_all(dir)?;Ok(())
    }
}
