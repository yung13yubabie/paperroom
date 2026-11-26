//! Opt-in policy for existing or newly mapped Markdown from one imported Vault.
use super::{Column,columns,snapshot_field,typed_prefix};
use crate::store::{Result,obsidian_base::{self,Base,View,FolderRule},collections::MAX_ITEMS};
use rusqlite::{Transaction,params};


pub(crate) const SCHEMA:&str="CREATE TABLE IF NOT EXISTS obsidian_base_scope(
 collection_id INTEGER PRIMARY KEY REFERENCES obsidian_base_dynamic(collection_id) ON DELETE CASCADE,
 paths TEXT NOT NULL, fields TEXT NOT NULL);";

enum Field{FileName,Folder,Path,Ext,Front(String),FileTest(String),Linked}

impl Field {
    fn encode(&self)->(&'static str,&str){match self{
        Self::FileName=>("name",""),Self::Folder=>("folder",""),Self::Path=>("path",""),Self::Ext=>("ext",""),
        Self::Front(k)=>("front",k),Self::FileTest(e)=>("file_test",e),Self::Linked=>("linked","")}}
    fn decode(tag:&str,arg:String)->Result<Self>{Ok(match tag{"name"=>Self::FileName,"folder"=>Self::Folder,"path"=>Self::Path,
        "ext"=>Self::Ext,"front"=>Self::Front(arg),"file_test"=>Self::FileTest(arg),"linked"=>Self::Linked,_=>return Err("來源欄位設定無效。".into())})}
}

pub(super) fn register(tx:&Transaction<'_>,root:i64,col:i64,view:&View,base:&Base)->Result<()>{
    let count:i64=tx.query_row("SELECT count(*) FROM obsidian_base_scope s JOIN obsidian_base_dynamic b USING(collection_id) WHERE b.import_root=?1",[root],|r|r.get(0))?;
    if count>=128{return Err("每個 Vault 最多連結 128 個可接納新來源的集合。".into());}
    let fields:Vec<_>=columns(base,view)?.into_iter().map(|(label,c)|{
        let f=match c{Column::Filename=>Field::FileName,Column::Folder=>Field::Folder,Column::Path=>Field::Path,Column::Extension=>Field::Ext,
            Column::Front(k) if k=="status" || base.types.contains_key(&k)=>Field::Linked,Column::Front(k)=>Field::Front(k),
            Column::Formula(e)=>e.path_expression().map_or(Field::Linked,Field::FileTest)};
        (label,f)
    }).collect();
    let paths=view.folders.iter().map(|r|match r{FolderRule::Exact(s)=>("exact",s),FolderRule::Path(s)=>("path",s),FolderRule::Subtree(s)=>("subtree",s)}).collect::<Vec<_>>();
    let fields=fields.iter().map(|(label,f)|(label,f.encode())).collect::<Vec<_>>();
    tx.execute("INSERT INTO obsidian_base_scope(collection_id,paths,fields) VALUES(?1,?2,?3)",params![col,serde_json::to_string(&paths)?,serde_json::to_string(&fields)?])?;
    Ok(())
}

pub(crate) fn mapped(tx:&Transaction<'_>,root:i64,note:i64)->Result<()>{
    let path:String=tx.query_row("SELECT p.relative_path FROM vault_paths p JOIN notes n ON n.id=p.note_id JOIN notes r ON r.id=p.import_root WHERE p.import_root=?1 AND p.note_id=?2 AND n.trash_root IS NULL AND r.trash_root IS NULL",params![root,note],|r|r.get(0))?;
    let mut q=tx.prepare("SELECT s.collection_id,s.paths,s.fields FROM obsidian_base_scope s JOIN obsidian_base_dynamic b USING(collection_id) WHERE b.import_root=?1 LIMIT 129")?;
    let configs=q.query_map([root],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if configs.len()>128{return Err("Vault 的即時檢視數量超過上限。".into());}
    let (folder,name)=path.rsplit_once('/').unwrap_or(("",&path));
    let mut budget=32*1024*1024;
    for (col,paths,fields) in configs{
        let paths=serde_json::from_str::<Vec<(String,String)>>(&paths)?.into_iter().map(|(tag,s)|match tag.as_str(){
            "exact"=>Ok(FolderRule::Exact(s)),"path"=>Ok(FolderRule::Path(s)),"subtree"=>Ok(FolderRule::Subtree(s)),_=>Err("來源路徑條件無效。")}).collect::<std::result::Result<Vec<_>,_>>()?;
        if !obsidian_base::matches_paths(&paths,&path){
            tx.execute("DELETE FROM collection_notes WHERE collection_id=?1 AND note_id=?2",params![col,note])?;
            continue;
        }
        let exists=tx.prepare_cached("SELECT 1 FROM collection_notes WHERE collection_id=?1 AND note_id=?2")?.exists(params![col,note])?;
        if !exists {
            let count:i64=tx.query_row("SELECT count(*) FROM collection_notes WHERE collection_id=?1",[col],|r|r.get(0))?;
            if count>=MAX_ITEMS as i64{return Err("來源變動會讓集合超過 1,000 篇；整筆變更已取消。".into());}
            tx.execute("INSERT INTO collection_notes(collection_id,note_id,status) VALUES(?1,?2,'已匯入')",params![col,note])?;
        }
        tx.execute("INSERT INTO obsidian_base_candidates(collection_id,note_id,relative_path) VALUES(?1,?2,?3) ON CONFLICT(collection_id,note_id) DO UPDATE SET relative_path=excluded.relative_path",params![col,note,&path])?;
        let fields=serde_json::from_str::<Vec<(String,(String,String))>>(&fields)?.into_iter().map(|(label,(tag,arg))|
            Ok((label,Field::decode(&tag,arg)?))).collect::<Result<Vec<_>>>()?;
        let selected=fields.iter().filter_map(|(_,f)|if let Field::Front(k)=f{Some(k.as_str())}else{None}).collect::<std::collections::BTreeSet<_>>();
        // Existing frontmatter snapshots follow their original import/admission; only pathname-derived cells change on move.
        let front=if !exists && !selected.is_empty(){
            obsidian_base::front_fields(typed_prefix(tx,note,&mut budget)?.as_bytes(),&selected)?
        }else{Default::default()};
        for (label,field) in fields{
            let value=match &field{
                Field::FileName=>name,Field::Folder=>folder,Field::Path=>&path,
                Field::Ext=>name.rsplit_once('.').map_or("",|(_,ext)|ext),
                Field::Linked=>"",Field::Front(k)=>{if exists{continue}match front.get(k){Some(v)=>v,None=>continue}},
                Field::FileTest(expr)=>{let f=obsidian_base::FileFormula::parse(expr)?;if f.path_expression().is_none(){return Err("檔案欄位公式失效。".into())}f.display(&path,folder,name)}
            };
            tx.execute("DELETE FROM obsidian_base_file_values WHERE collection_id=?1 AND note_id=?2 AND key=?3",params![col,note,label])?;
            snapshot_field(tx,col,note,&label,value)?;
        }
    }
    Ok(())
}
