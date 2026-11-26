//! Explicitly dynamic Bases imports: persist bounded candidate IDs and rules, project without writes.
use super::{Result,Row,obsidian_base,obsidian_base_typed::{self,Test},read_row};
use rusqlite::{Connection,OptionalExtension,params};
use std::collections::BTreeMap;

pub(super) const SCHEMA:&str="
CREATE TABLE IF NOT EXISTS obsidian_base_dynamic(
 collection_id INTEGER PRIMARY KEY REFERENCES collections(id) ON DELETE CASCADE,
 import_root INTEGER REFERENCES notes(id) ON DELETE SET NULL,
 predicates TEXT NOT NULL, sorts TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS obsidian_base_candidates(
 collection_id INTEGER NOT NULL REFERENCES obsidian_base_dynamic(collection_id) ON DELETE CASCADE,
 note_id INTEGER NOT NULL, relative_path TEXT NOT NULL,
 PRIMARY KEY(collection_id,note_id), FOREIGN KEY(collection_id,note_id)
 REFERENCES collection_notes(collection_id,note_id) ON DELETE CASCADE);";

type Sort=(String,String,bool);
pub(super) fn register(db:&Connection,col:i64,root:i64,view:&obsidian_base::View,base:&obsidian_base::Base)->Result<()>{
    let predicates:Vec<_>=view.typed_filters.iter().map(|t|format!("note.{} {} {}",t.key,t.op,t.operand)).collect();
    let sorts:Vec<_>=view.sorts.iter().map(|(key,desc)| (key.as_str(),base.types[key],desc)).collect();
    let sort_plan = match view.limit {
        Some(limit) => serde_json::json!({"sorts":sorts,"limit":limit}).to_string(),
        None => serde_json::to_string(&sorts)?,
    };
    db.execute("INSERT INTO obsidian_base_dynamic(collection_id,import_root,predicates,sorts) VALUES(?1,?2,?3,?4)",params![col,root,serde_json::to_string(&predicates)?,sort_plan])?;
    Ok(())
}

struct Plan{tests:Vec<Test>,sorts:Vec<Sort>,limit:Option<usize>}
fn plan(db:&Connection,col:i64)->Result<Option<Plan>>{
    let raw:Option<(String,String)>=db.query_row("SELECT predicates,sorts FROM obsidian_base_dynamic WHERE collection_id=?1",[col],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((p,s))=raw else{return Ok(None)};
    if p.len()>32768 || s.len()>4096{return Err("動態 Bases 設定超過上限。".into())}
    let exprs:Vec<String>=serde_json::from_str(&p)?;
    let config:serde_json::Value=serde_json::from_str(&s)?;
    let (sorts,limit):(Vec<Sort>,Option<usize>)=match config {
        serde_json::Value::Object(mut object) => {
            let limit=object.remove("limit").and_then(|v|v.as_u64()).filter(|n|(1..=1000).contains(n)).ok_or("動態檢視的筆數上限無效。")?;
            let sorts=object.remove("sorts").ok_or("缺少排序設定。")?;
            if !object.is_empty(){return Err("未知的動態排序設定。".into())}
            (serde_json::from_value(sorts)?,Some(limit as usize))
        }
        legacy => (serde_json::from_value(legacy)?,None),
    };
    if exprs.len()>8 || sorts.len()>4 || sorts.iter().any(|(_,kind,_)|!matches!(kind.as_str(),"number"|"boolean")) {return Err("動態 Bases 規則無效。".into())}
    let tests=exprs.iter().map(|expr|Test::parse(expr).ok_or("無效的型別條件。")).collect::<std::result::Result<_,_>>()?;
    Ok(Some(Plan{tests,sorts,limit}))
}

const SOURCE:&str=" FROM obsidian_base_candidates c
 JOIN obsidian_base_dynamic b ON b.collection_id=c.collection_id
 JOIN notes root ON root.id=b.import_root AND root.trash_root IS NULL
 JOIN vault_paths p ON p.note_id=c.note_id AND p.import_root=b.import_root AND p.relative_path=c.relative_path
 JOIN collection_notes m ON m.collection_id=c.collection_id AND m.note_id=c.note_id
 JOIN notes n ON n.id=c.note_id AND n.trash_root IS NULL WHERE c.collection_id=?1";

impl Plan{
    fn evaluate(&self,bytes:&[u8],budget:&mut usize)->Result<Option<Vec<(u8,i128)>>>{
        let len=std::str::from_utf8(bytes).map_or_else(|e|e.valid_up_to(),|s|s.len());
        let prefix=std::str::from_utf8(&bytes[..len])?;
        let mut fields=BTreeMap::new();
        for (k,kind) in self.tests.iter().map(|t|(t.key.as_str(),t.kind)).chain(self.sorts.iter().map(|(k,kind,_)|(k.as_str(),kind.as_str()))){
            if !fields.contains_key(k){
                *budget=budget.checked_sub(bytes.len()).ok_or("動態 Bases 頁首讀取量超過 32 MiB。")?;
                fields.insert(k,obsidian_base_typed::snapshot(prefix,k,kind,None).ok());
            }
        }
        if !self.tests.iter().all(|t|fields.get(t.key.as_str()).and_then(|v|v.as_ref()).and_then(|s|t.matches(s))==Some(true)){
            return Ok(None)
        }
        let keys=self.sorts.iter().map(|(k,kind,desc)|{
            let value=fields[k.as_str()].as_ref();
            let (rank,num)=if value.is_some(){obsidian_base_typed::group(value,kind).0}else{(3,0)};
            (rank,if *desc && rank==0{-num}else{num})
        }).collect();
        Ok(Some(keys))
    }
}

pub(super) fn rows(db:&Connection,col:i64)->Result<Option<Vec<(Row,String)>>>{
    let tx=db.is_autocommit().then(||db.unchecked_transaction()).transpose()?;
    let Some(plan)=plan(db,col)? else{return Ok(None)};
    let mut stmt=db.prepare(&format!("SELECT n.id,n.parent,n.title,n.pinned,n.trash_root,m.status,c.relative_path,substr(CAST(n.body AS BLOB),1,?2){SOURCE} LIMIT 1001"))?;
    let mut reader=stmt.query(params![col,(obsidian_base::FRONT_LIMIT+4)as i64])?;
    let mut visible=Vec::new();let mut budget=32*1024*1024usize;let mut count=0;
    while let Some(r)=reader.next()?{
        count+=1;if count>1000{return Err("候選筆記超過 1,000 篇。".into())}
        if let Some(keys)=plan.evaluate(r.get_ref(7)?.as_blob()?,&mut budget)?{
            visible.push((keys,r.get::<_,String>(6)?,read_row(r)?,r.get(5)?));
        }
    }
    drop(reader);drop(stmt);
    visible.sort_by(|a,b|a.0.cmp(&b.0).then_with(||a.1.cmp(&b.1)).then_with(||a.2.id.cmp(&b.2.id)));
    if let Some(limit)=plan.limit{visible.truncate(limit);}
    if let Some(tx)=tx{tx.commit()?}
    Ok(Some(visible.into_iter().map(|(_,_,r,status)|(r,status)).collect()))
}

pub(super) fn require_visible(db:&Connection,col:i64,note:i64)->Result<()> {
    let Some(plan)=plan(db,col)? else{return Ok(())};
    if plan.limit.is_some() {
        // Membership depends on every sorted candidate, including edits by another connection.
        let visible=rows(db,col)?.ok_or("找不到動態檢視。")?;
        return if visible.iter().any(|(row,_)|row.id==note){Ok(())}else{Err("這篇來源已不在動態檢視顯示的範圍。".into())};
    }
    let bytes:Option<Vec<u8>>=db.query_row(&format!("SELECT substr(CAST(n.body AS BLOB),1,?3){SOURCE} AND n.id=?2"),params![col,note,(obsidian_base::FRONT_LIMIT+4)as i64],|r|r.get(0)).optional()?;
    let mut budget=32*1024*1024usize;
    if let Some(bytes)=bytes && plan.evaluate(&bytes,&mut budget)?.is_some(){return Ok(())}
    Err("這篇來源已不在動態檢視內。請從原筆記重新開啟。".into())
}

pub(super) fn visible_ids(db:&Connection,col:i64)->Result<Option<std::collections::HashSet<i64>>>{
    Ok(rows(db,col)?.map(|rows|rows.into_iter().map(|(row,_)|row.id).collect()))
}

pub(super) fn require_manual_membership(db:&Connection,col:i64)->Result<()> {
    if db.prepare_cached("SELECT 1 FROM obsidian_base_dynamic WHERE collection_id=?1")?.exists([col])? {
        return Err("動態 Bases 的候選由來源 Vault 與匯入時的路徑決定，不能在集合中新增、移除或搬動資料列。請編輯來源欄位。".into());
    }
    Ok(())
}
