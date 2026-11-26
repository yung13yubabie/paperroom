//! Linked YAML booleans and bounded base-ten numbers. No floating-point comparisons.
use super::{Notebook,Result,obsidian_base,obsidian_base_live};
use rusqlite::{Connection,OptionalExtension,Transaction,TransactionBehavior,params};
use std::{collections::{BTreeSet,HashMap},ops::Range};

pub(super) const SCHEMA:&str="CREATE TABLE IF NOT EXISTS obsidian_base_typed_columns(
 collection_id INTEGER NOT NULL,key TEXT NOT NULL,import_root INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
 source_key TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('boolean','number')),op TEXT,operand TEXT,
 CHECK ((op IS NULL)=(operand IS NULL)), PRIMARY KEY(collection_id,key),
 FOREIGN KEY(collection_id,key) REFERENCES obsidian_base_file_columns(collection_id,key) ON DELETE CASCADE);
 CREATE TABLE IF NOT EXISTS obsidian_base_typed_groups(view_id INTEGER PRIMARY KEY REFERENCES collection_views(id) ON DELETE CASCADE,
 label TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('boolean','number')));";

// Rows exist only for descending imported views. Existing groups keep their original order.
pub(super) const DESCENDING_GROUP_SCHEMA:&str="CREATE TABLE IF NOT EXISTS obsidian_base_descending_groups(
 view_id INTEGER PRIMARY KEY REFERENCES obsidian_base_typed_groups(view_id) ON DELETE CASCADE);";

#[derive(Debug,Clone,PartialEq,Eq)]
pub(super) struct Test { pub key:String, pub op:String, pub operand:String, pub kind:&'static str }

fn number(s:&str)->Option<i128>{
    let digits=s.strip_prefix('-').unwrap_or(s);
    let (whole,frac)=digits.split_once('.').unwrap_or((digits,""));
    if whole.is_empty() || whole.len()>15 || frac.len()>6 || digits.ends_with('.') ||
        (whole.len()>1 && whole.starts_with('0')) || !whole.bytes().chain(frac.bytes()).all(|b|b.is_ascii_digit()) {return None}
    let fraction=if frac.is_empty(){0}else{frac.parse::<i128>().ok()?*10i128.pow((6-frac.len()) as u32)};
    let scaled=whole.parse::<i128>().ok()?*1_000_000+fraction;
    Some(if s.starts_with('-'){-scaled}else{scaled})
}

impl Test {
    pub fn parse(expr:&str)->Option<Self>{
        for op in [">=","<=","==","!=",">","<"]{
            let Some((lhs,rhs))=expr.split_once(op) else{continue};
            let lhs=lhs.trim();let key=lhs.strip_prefix("note.").unwrap_or(lhs);
            if key=="status" || !matches!(yaml_rust2::Yaml::from_str(key),yaml_rust2::Yaml::String(_)) || key.is_empty() || key.len()>128 || !key.chars().all(|c|c.is_alphanumeric()||matches!(c,'_'|'-')) {return None}
            let rhs=rhs.trim();
            let kind=if matches!(rhs,"true"|"false") && matches!(op,"=="|"!=") {"boolean"}
                else if number(rhs).is_some() {"number"}else{return None};
            return Some(Self{key:key.into(),op:op.into(),operand:rhs.into(),kind});
        }
        None
    }
    pub fn matches(&self,raw:&str)->Option<bool>{
        let cmp=match self.kind {
            "boolean" if matches!(raw,"true"|"false")=>raw.cmp(self.operand.as_str()),
            "number"=>number(raw)?.cmp(&number(&self.operand)?),
            _=>return None
        };
        Some(match self.op.as_str(){"=="=>cmp.is_eq(),"!="=>cmp.is_ne(),">="=>cmp.is_ge(),"<="=>cmp.is_le(),">"=>cmp.is_gt(),"<"=>cmp.is_lt(),_=>return None})
    }
}

const MISSING:&str="（缺少）";
const INVALID:&str="（型別或格式不符）";
fn allowed(s:&str,kind:&str)->bool{s=="null" || match kind{"boolean"=>matches!(s,"true"|"false"),"number"=>number(s).is_some(),_=>false}}
struct Cell { value:String, range:Range<usize>, before:String, after:String }
fn cell(body:&str,key:&str,kind:&str)->Result<Cell>{
    let raw=body.strip_prefix('\u{feff}').unwrap_or(body);
    let bom=body.len()-raw.len();
    let eol=if raw.split_once('\n').is_some_and(|(first,_)|first.ends_with('\r')){"\r\n"}else{"\n"};
    let bytes=body.as_bytes();
    let seen=obsidian_base::front_fields_with_null(&bytes[..bytes.len().min(obsidian_base::FRONT_LIMIT+4)],&BTreeSet::from([key]))?;
    let mut lines=raw.split_inclusive('\n');
    let first=lines.next().unwrap_or("");
    if first.trim_end_matches(['\r','\n'])!="---" {
        return Ok(Cell{value:MISSING.into(),range:bom..bom,before:format!("---{eol}{key}: "),after:format!("{eol}---{eol}")})
    }
    let mut offset=bom+first.len();
    let opening_end=offset;
    for line in lines{
        let text=line.trim_end_matches(['\r','\n']);
        if text=="---"{break}
        if let Some(rhs)=text.strip_prefix(key).and_then(|s|s.strip_prefix(':')){
            let spaced=rhs.trim_start_matches(' ');
            if !rhs.is_empty() && rhs==spaced {return Err("鍵名後需要空格。".into())}
            let comment=spaced.char_indices().find(|(i,ch)|*ch=='#' && (*i==0 || spaced.as_bytes()[i-1]==b' ')).map(|(i,_)|i);
            let token=comment.map_or(spaced,|end|&spaced[..end]).trim_end_matches(' ');
            let start=offset+text.len()-spaced.len();
            let canonical=if matches!(token,""|"~"|"null"|"Null"|"NULL"){"null"}else{token};
            let matches_decoded=seen.get(key).is_some_and(|v|if canonical=="null"{matches!(v.as_str(),""|"~"|"null"|"Null"|"NULL")}else{v==canonical});
            if !allowed(canonical,kind) || !matches_decoded{return Err("來源不是受支援的單行 YAML 型別。".into())}
            return Ok(Cell{value:canonical.into(),range:start..start+token.len(),before:if rhs.is_empty(){" ".into()}else{String::new()},after:if comment==Some(0){" ".into()}else{String::new()}});
        }
        offset+=line.len();
    }
    if seen.contains_key(key){return Err("只支援無縮排、未引用的頂層來源鍵。".into())}
    Ok(Cell{value:MISSING.into(),range:opening_end..opening_end,before:format!("{key}: "),after:eol.into()})
}

pub(super) fn snapshot(prefix:&str,key:&str,kind:&str,test:Option<&Test>)->Result<String>{
    let value=cell(prefix,key,kind)?.value;
    Ok(test.and_then(|t|t.matches(&value)).map_or(value,|b|b.to_string()))
}
pub(super) fn filter(prefix:&str,test:&Test)->Result<bool>{
    Ok(test.matches(&cell(prefix,&test.key,test.kind)?.value)==Some(true))
}
const ROWS:&str=" FROM obsidian_base_typed_columns b JOIN obsidian_base_file_values v ON (v.collection_id,v.key)=(b.collection_id,b.key)
 JOIN vault_paths p ON p.note_id=v.note_id AND p.import_root=b.import_root JOIN notes root ON root.id=b.import_root AND root.trash_root IS NULL
 JOIN notes n ON n.id=v.note_id AND n.trash_root IS NULL WHERE b.collection_id=?1 AND b.key=?2";

pub(super) fn display(db:&Connection,col:i64,key:&str,editable:bool)->Result<Option<HashMap<i64,String>>>{
    let binding:Option<(String,String,Option<String>,Option<String>)>=db.query_row("SELECT source_key,kind,op,operand FROM obsidian_base_typed_columns WHERE collection_id=?1 AND key=?2",params![col,key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((source,kind,op,operand))=binding else{return Ok(None)};
    let test=op.zip(operand).map(|(op,operand)|Test{key:source.clone(),op,operand,kind:if kind=="number"{"number"}else{"boolean"}});
    let mut result=HashMap::new();
    if editable && test.is_some(){return Ok(Some(result))}
    let mut q=db.prepare(&format!("SELECT n.id,substr(CAST(n.body AS BLOB),1,?3){ROWS} LIMIT 1001"))?;
    let mut rows=q.query(params![col,key,(obsidian_base::FRONT_LIMIT+4)as i64])?;
    let mut budget=32*1024*1024usize;
    while let Some(row)=rows.next()?{
        let bytes=row.get_ref(1)?.as_blob()?;budget=budget.checked_sub(bytes.len()).ok_or("即時欄位超過讀取上限。")?;
        let n=std::str::from_utf8(bytes).map_or_else(|e|e.valid_up_to(),|s|s.len());
        let value=match cell(std::str::from_utf8(&bytes[..n])?,&source,&kind){
            Ok(cell)=>test.as_ref().and_then(|p|p.matches(&cell.value)).map_or(cell.value,|b|b.to_string()),
            Err(_) if editable=>continue, Err(_)=>INVALID.into()
        };
        result.insert(row.get(0)?,value);
    }
    Ok(Some(result))
}
impl Notebook{
    pub(crate) fn typed_base_keys(&self,col:i64)->Result<Vec<(String,String)>>{
        Ok(self.connection.prepare("SELECT key,kind FROM obsidian_base_typed_columns WHERE collection_id=?1 AND op IS NULL ORDER BY key")?
            .query_map([col],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }
    pub(crate) fn typed_base_rows(&self,col:i64,key:&str)->Result<HashMap<i64,String>>{
        let mut values=display(&self.connection,col,key,true)?.unwrap_or_default();
        if let Some(ids)=super::obsidian_base_dynamic::visible_ids(&self.connection,col)?{values.retain(|id,_|ids.contains(id));}
        Ok(values)
    }
    pub(crate) fn typed_base_current(&self,col:i64,key:&str,id:i64)->Result<(i64,String)>{
        super::obsidian_base_dynamic::require_visible(&self.connection,col,id)?;
        let (version,body,source,kind):(i64,String,String,String)=self.connection.query_row(&format!("SELECT n.version,n.body,b.source_key,b.kind{ROWS} AND n.id=?3 AND b.op IS NULL"),params![col,key,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        Ok((version,cell(&body,&source,&kind)?.value))
    }
    pub(crate) fn edit_typed_base(&self,col:i64,key:&str,id:i64,expected:i64,new:&str)->Result<()>{
        let tx=Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        edit_typed_base_on(&tx,col,key,id,expected,new)?;
        tx.commit()?;Ok(())
    }
}


pub(super) fn edit_typed_base_on(tx:&Transaction<'_>,col:i64,key:&str,id:i64,expected:i64,new:&str)->Result<()> {
        super::obsidian_base_dynamic::require_visible(tx,col,id)?;
        let (title,body,source,kind):(String,String,String,String)=tx.query_row(&format!("SELECT n.title,n.body,b.source_key,b.kind{ROWS} AND n.id=?3 AND n.version=?4 AND b.op IS NULL"),params![col,key,id,expected],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or("來源或版本已變更。")?;
        if !allowed(new,&kind){return Err("請輸入 true、false、限定範圍的十進位數或 null，符合選取欄位型別。".into())}
        let old=cell(&body,&source,&kind)?;
        if old.value==new{return Ok(())}
        let replacement=format!("{}{new}{}",old.before,old.after);
        obsidian_base_live::save_splice(tx,id,expected,&title,body,old.range,&replacement,|updated|{
            if cell(updated,&source,&kind)?.value!=new{return Err("新值未能原樣保存。".into())}Ok(())
        })?;
    Ok(())
}

pub(super) fn existing_direct_value(body:&str,source:&str,kind:&str)->Option<String>{
    cell(body,source,kind).ok().filter(|cell|cell.value!=MISSING).map(|cell|cell.value)
}

// The order key is independent from the caption: 2, 2.0 and 2.000 share a lane.
pub(super) fn group(value:Option<&String>,kind:&str)->((u8,i128),String){
    match value.map(String::as_str){
        Some("false") if kind=="boolean"=>((0,0),"false".into()),
        Some("true") if kind=="boolean"=>((0,1),"true".into()),
        Some(text) if kind=="number" && number(text).is_some()=>{
            let val=number(text).unwrap();
            let label=if val==0{"0".to_owned()}else if text.contains('.'){
                text.trim_end_matches('0').trim_end_matches('.').to_owned()
            }else{text.to_owned()};
            ((0,val),label)
        }
        None|Some(MISSING)=>((1,0),MISSING.into()),
        Some("null")=>((2,0),"null".into()),
        _=>((3,0),INVALID.into())
    }
}
