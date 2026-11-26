//! Explicitly copy one new local Markdown source into an existing, witnessed Vault.
use super::{Notebook, Result, insert, relocation};
use rusqlite::{Connection,TransactionBehavior,params};
use std::{collections::BTreeMap,path::{Path,PathBuf}};

#[derive(Eq,PartialEq)]
pub(crate) struct SourceAdmission {
    anchor:i64,root:i64,anchor_token:String,root_token:String,directory:PathBuf,
    parent:i64,parent_token:String,new_folders:Vec<String>,path:String,title:String,body:String
}
impl SourceAdmission {
    pub(crate) fn report(&self)->String{
        format!("加入 Vault #{} 的新筆記「{}」？\r\n來源：{}\r\n路徑：{}\r\nUTF-8 內容 {} 位元組。磁碟檔案不會寫入。",self.root,self.title,self.directory.display(),self.path,self.body.len())
    }
}

// Only a mapped file's verified tree ancestry can identify an existing imported folder.
// A similarly named ordinary note never becomes the parent of an admission.
fn ancestor_prefix(db:&Connection,root:i64,id:i64,dirs:&[&str],peer:&str)->Result<Vec<i64>>{
    let peer_parts=peer.split('/').collect::<Vec<_>>();
    let shared=dirs.iter().zip(&peer_parts[..peer_parts.len()-1]).take_while(|(a,b)|a==b).count();
    if shared==0{return Ok(Vec::new())}
    let mut chain=Vec::new();let mut current=id;
    for _ in 0..=64{
        let parent:Option<i64>=db.query_row("SELECT parent FROM notes WHERE id=?1",[current],|r|r.get(0))?;
        let Some(parent)=parent else{return Ok(Vec::new())};
        if parent==root {
            chain.reverse();
            let mut out=Vec::new();let mut owner=root;
            for ((id,title),expected) in chain.into_iter().zip(dirs.iter().take(shared)){
                if title!=*expected{break}
                let duplicate:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM notes WHERE parent=?1 AND trash_root IS NULL AND id<>?2 AND title=?3 COLLATE NOCASE)",params![owner,id,title],|r|r.get(0))?;
                if duplicate{return Err("資料夾階層包含同名項目，無法確認新增位置。".into())}
                owner=id;out.push(id);
            }
            return Ok(out)
        }
        let (title,eligible):(String,bool)=db.query_row("SELECT title,trash_root IS NULL AND NOT EXISTS(SELECT 1 FROM vault_paths WHERE note_id=n.id) FROM notes n WHERE id=?1",[parent],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if !eligible{return Ok(Vec::new())}
        chain.push((parent,title));current=parent;
    }
    Ok(Vec::new())
}

fn inspect(db:&Connection,anchor:i64,directory:&Path,path:&str)->Result<SourceAdmission>{
    relocation::safe_note_path(path)?;
    let (root,_)=relocation::origin(db,anchor)?;
    let (anchor_token,root_token):(String,String)=db.query_row("SELECT a.token,r.token FROM note_link_identities a,note_link_identities r WHERE a.note_id=?1 AND r.note_id=?2",params![anchor,root],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let verified:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM vault_push_verified WHERE note_id=?1) AND NOT EXISTS(SELECT 1 FROM heptabase_import_roots WHERE import_root=?2)",params![anchor,root],|r|r.get(0))?;
    if !verified{return Err("請從有已核對原檔的 Vault 來源筆記加入新檔。".into())}
    let scan=crate::vault::scan_for_sync(directory)?;
    let mut files=BTreeMap::new();
    for file in scan.pages{
        let relative=file.parts.join("/");
        if relative!=path && relocation::paths_conflict(&relative,path){return Err("來源路徑與另一個檔案的大小寫或階層衝突。".into())}
        if files.insert(relative,file).is_some(){return Err("來源資料夾包含重複路徑。".into())}
    }
    let dirs=path.rsplit_once('/').map(|(d,_)|d.split('/').collect::<Vec<_>>()).unwrap_or_default();
    let mut known=Vec::<i64>::new();
    let mut q=db.prepare("SELECT p.relative_path,n.trash_root IS NULL,n.id,b.body,v.note_id IS NOT NULL FROM vault_paths p JOIN notes n ON n.id=p.note_id
        LEFT JOIN vault_sync_base b ON b.note_id=n.id LEFT JOIN vault_push_verified v ON v.note_id=n.id WHERE p.import_root=?1 ORDER BY p.relative_path LIMIT 15001")?;
    let mut peers=q.query([root])?;let mut count=0;
    while let Some(row)=peers.next()?{
        count+=1;if count>=crate::archive::MAX_ENTRIES{return Err("Vault 配對數量已達上限。".into())}
        let relative=row.get_ref(0)?.as_str()?;
        if relocation::paths_conflict(relative,path){return Err("此路徑已被配對、在垃圾桶中，或與既有路徑衝突。".into())}
        if row.get::<_,bool>(1)?{
            let file=files.get(relative).ok_or("選取的資料夾缺少已配對的原檔；請先處理搬移或重新確認資料夾。")?;
            if !row.get::<_,bool>(4)? || row.get_ref(3)?.as_str().ok()!=Some(file.body.as_str()){
                return Err("資料夾與已保存的 Vault 原文或核對狀態不符。請確認資料夾或先同步。".into())
            }
            let chain=ancestor_prefix(db,root,row.get(2)?,&dirs,relative)?;
            if chain.iter().zip(&known).any(|(a,b)|a!=b){return Err("來源目錄對應多個工作區位置，無法辨識。".into())}
            if chain.len()>known.len(){known=chain}
        }
    }
    let parent=*known.last().unwrap_or(&root);
    let new_folders=dirs[known.len()..].iter().map(|s|s.to_string()).collect::<Vec<_>>();
    if let Some(first)=new_folders.first() && db.prepare("SELECT 1 FROM notes WHERE parent=?1 AND trash_root IS NULL AND title=?2 COLLATE NOCASE")?.exists(params![parent,first])?{
        return Err("工作區已有同名但未能由來源筆記核對的資料夾，未加入。".into())
    }
    let parent_token=db.query_row("SELECT token FROM note_link_identities WHERE note_id=?1",[parent],|r|r.get(0))?;
    let file=files.remove(path).ok_or("沒有找到完全符合路徑、可安全讀取的 UTF-8 Markdown。")?;
    Ok(SourceAdmission{anchor,root,anchor_token,root_token,parent,parent_token,new_folders,directory:directory.into(),path:path.into(),title:file.title,body:file.body})
}

impl Notebook{
    pub(crate) fn preview_vault_admission(&self,anchor:i64,directory:&Path,path:&str)->Result<SourceAdmission>{
        let tx=self.connection.unchecked_transaction()?;let preview=inspect(&tx,anchor,directory,path)?;tx.commit()?;Ok(preview)
    }
    pub(crate) fn apply_vault_admission(&mut self,plan:&SourceAdmission)->Result<i64>{
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if inspect(&tx,plan.anchor,&plan.directory,&plan.path)?!=*plan{return Err("已核對的來源或檔案內容改變，請重新預覽。".into())}
        let mut parent=plan.parent;
        for folder in &plan.new_folders{parent=insert(&tx,Some(parent),folder,"")?;}
        let id=insert(&tx,Some(parent),&plan.title,&plan.body)?;
        tx.execute("INSERT INTO vault_paths(note_id,import_root,relative_path) VALUES(?1,?2,?3)",params![id,plan.root,plan.path])?;
        tx.execute("INSERT INTO vault_sync_base(note_id,body) VALUES(?1,?2)",params![id,plan.body])?;
        tx.execute("INSERT INTO vault_push_verified(note_id) VALUES(?1)",[id])?;
        super::super::obsidian_base_import::scope::mapped(&tx,plan.root,id)?;
        tx.commit()?;Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn witnessed_source_add_relocation_and_reopen_update_only_opted_in_vault_views()->Result<()>{
        let dir=std::env::temp_dir().join(format!("paperroom-vault-scope-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        let vault=dir.join("source");let other=dir.join("other");
        fn body(name:&str,cost:i32)->String{format!("---\r\napproved: true\r\ncost: {cost}\r\n---\r\nEvidence {name}.")}
        for folder in [&vault,&other]{
            fs::create_dir_all(folder.join("Active"))?;fs::create_dir(folder.join("Archive"))?;
            fs::write(folder.join("Active/A.md"),body("anchor",3))?;
            fs::write(folder.join("Archive/B.md"),body(if folder==&vault{"peer B"}else{"foreign B"},2))?;
        }
        let db_path=dir.join("workspace.sqlite");
        let mut db=Notebook::open(&db_path,false)?;
        let root=db.import_vault("來源",crate::vault::read(&vault)?.pages)?;
        let foreign=db.import_vault("同名來源",crate::vault::read(&other)?.pages)?;
        fn mapped(db:&Notebook,root:i64,path:&str)->Result<i64>{Ok(db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",params![root,path],|r|r.get(0))?)}
        let anchor=mapped(&db,root,"Active/A.md")?;let peer=mapped(&db,root,"Archive/B.md")?;
        let raw=r#"formulas: {costCheck: 'note.cost >= 0', nested: 'file.path.contains("Sub/")'}
views:
  - type: table
    name: ActiveFolder
    filters: {and: ['file.inFolder("Active")', 'note.approved == true']}
    order: [file.path, approved, cost, formula.nested]
    sort: [{property: note.cost, direction: ASC}]
  - type: table
    name: MatchExact
    filters: 'file.path == "Active/New.md"'
    order: [file.name]
"#;
        let (_,fixed)=db.import_obsidian_base_options(root,"fixed.base",raw.as_bytes().to_vec(),true,true)?;
        let (_,scoped)=db.import_vault_scope_base(root,"scoped.base",raw.as_bytes().to_vec())?;
        let (_,foreign_scope)=db.import_vault_scope_base(foreign,"foreign.base",raw.as_bytes().to_vec())?;
        fn ids(db:&Notebook,col:i64)->Result<Vec<i64>>{Ok(db.collection_rows(col)?.into_iter().map(|(r,_)|r.id).collect())}
        let sc=scoped[0].0;let exact=scoped[1].0;
        let a_folder=db.note(anchor)?.unwrap().row.parent.unwrap();
        for folder in [&vault,&other]{fs::write(folder.join("Active/New.md"),body("new",1))?;}
        assert!(db.preview_vault_admission(anchor,&other,"Active/New.md").is_err());
        let old=db.preview_vault_admission(anchor,&vault,"Active/New.md")?;
        fs::write(vault.join("Active/New.md"),body("changed",1))?;
        assert!(db.apply_vault_admission(&old).is_err());assert_eq!(ids(&db,sc)?,[anchor]);
        let new_bytes=body("changed",1);
        let preview=db.preview_vault_admission(anchor,&vault,"Active/New.md")?;
        let new=db.apply_vault_admission(&preview)?;
        assert!(db.apply_vault_admission(&preview).is_err());
        assert_eq!(db.note(new)?.unwrap().row.parent,Some(a_folder));
        assert_eq!(db.note(new)?.unwrap().body,new_bytes);
        assert_eq!(fs::read(vault.join("Active/New.md"))?,new_bytes.as_bytes());
        assert_eq!(ids(&db,sc)?,[new,anchor]); assert_eq!(ids(&db,exact)?,[new]);
        assert_eq!(ids(&db,fixed[0].0)?,[anchor]);assert!(ids(&db,fixed[1].0)?.is_empty());
        assert_eq!(ids(&db,foreign_scope[0].0)?,[mapped(&db,foreign,"Active/A.md")?]);
        fs::create_dir(vault.join("Active/Sub"))?;
        fs::rename(vault.join("Archive/B.md"),vault.join("Active/Sub/B.md"))?;
        let plan=db.preview_vault_relocation(peer,&vault,"Active/Sub/B.md")?;
        db.apply_vault_relocation(&plan,&dir.join("pre-move.sqlite"))?;
        assert_eq!(ids(&db,sc)?,[new,peer,anchor]);
        assert_eq!(&db.collection_display_values(sc,"file.path")?[&peer],"Active/Sub/B.md");
        assert_eq!(&db.collection_display_values(sc,"nested")?[&peer],"true");
        fs::rename(vault.join("Active/New.md"),vault.join("Archive/Renamed.md"))?;
        let plan=db.preview_vault_relocation(new,&vault,"Archive/Renamed.md")?;
        db.apply_vault_relocation(&plan,&dir.join("pre-rename.sqlite"))?;
        assert_eq!(mapped(&db,root,"Archive/Renamed.md")?,new);
        assert_eq!(ids(&db,sc)?,[peer,anchor]);assert!(ids(&db,exact)?.is_empty());
        db.trash(peer)?;assert_eq!(ids(&db,sc)?,[anchor]);db.restore(peer)?;assert_eq!(ids(&db,sc)?,[peer,anchor]);
        fs::write(vault.join("Active/New.md"),body("replacement",0))?;
        let plan=db.preview_vault_admission(anchor,&vault,"Active/New.md")?;
        let replacement=db.apply_vault_admission(&plan)?;assert_ne!(replacement,new);
        assert_eq!(ids(&db,exact)?,[replacement]);assert_eq!(ids(&db,sc)?,[replacement,peer,anchor]);
        let folder=vault.join("Fresh/Sub");fs::create_dir_all(&folder)?;
        fs::write(folder.join("New.md"),body("new nested",2))?;
        let ambiguous=db.add(Some(root),"fresh","")?;
        assert!(db.preview_vault_admission(anchor,&vault,"Fresh/Sub/New.md").is_err());
        db.trash(ambiguous)?;
        let draft=db.preview_vault_admission(anchor,&vault,"Fresh/Sub/New.md")?;
        let nested=db.apply_vault_admission(&draft)?;
        let sub=db.note(nested)?.unwrap().row.parent.unwrap();let fresh=db.note(sub)?.unwrap().row.parent.unwrap();
        assert_eq!(db.note(sub)?.unwrap().row.title,"Sub");assert_eq!(db.note(fresh)?.unwrap().row.title,"Fresh");
        assert_eq!(db.note(fresh)?.unwrap().row.parent,Some(root));
        let backup=dir.join("backup.sqlite");db.backup(&backup)?;drop(db);
        for f in [&db_path,&backup]{let reopened=Notebook::open(f,false)?;
            assert_eq!(ids(&reopened,sc)?,[replacement,peer,anchor]);
            assert_eq!(ids(&reopened,exact)?,[replacement]);
            assert_eq!(ids(&reopened,fixed[0].0)?,[anchor]);
        }
        fs::remove_dir_all(dir)?;Ok(())
    }
}
