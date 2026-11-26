//! Explicit bindings for a saved, top-level Markdown status. Earlier imports remain snapshots.
use super::{Notebook, Result, note_highlights, obsidian_base, save_in};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::{collections::{BTreeSet, HashMap}, ops::Range};

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS obsidian_base_live_columns(
        collection_id INTEGER NOT NULL, key TEXT NOT NULL,
        import_root INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
        equals_text TEXT,
        PRIMARY KEY(collection_id,key),
        FOREIGN KEY(collection_id,key) REFERENCES obsidian_base_file_columns(collection_id,key) ON DELETE CASCADE
    );";

// Parse the whole YAML header first, then admit only a single, explicit, replaceable token.
// The raw lexeme's decoded string must also equal the independently parsed top-level value.
fn status_token(body: &str) -> Result<(Range<usize>, String)> {
    let bytes=body.as_bytes();
    let selected=BTreeSet::from(["status"]);
    let fields=obsidian_base::front_fields(&bytes[..bytes.len().min(obsidian_base::FRONT_LIMIT+4)],&selected)?;
    let decoded=fields.get("status").ok_or("找不到可編輯的頂層文字 status。")?;
    let raw=body.strip_prefix('\u{feff}').unwrap_or(body);
    let mut offset=body.len()-raw.len();
    let mut lines=raw.split_inclusive('\n');
    offset+=lines.next().ok_or("缺少 YAML 頁首。")?.len();
    for line in lines {
        let content=line.trim_end_matches(['\r','\n']);
        if content=="---" {break}
        if let Some(after)=content.strip_prefix("status:") {
            let leading=after.len()-after.trim_start_matches(' ').len();
            if leading==0 {return Err("status 需要鍵名後的空格。".into());}
            let source=after[leading..].trim_end_matches(' ');
            let checked=if source.starts_with('"') {serde_json::from_str::<String>(source).ok()}
                else if source.chars().all(|c| c.is_alphanumeric()||matches!(c,'_'|'-'|' '))
                    && matches!(yaml_rust2::Yaml::from_str(source),yaml_rust2::Yaml::String(_)) {
                    Some(source.to_owned())
                } else {None};
            if checked.as_ref()!=Some(decoded) {break}
            let start=offset+"status:".len()+leading;
            return Ok((start..start+source.len(),decoded.clone()));
        }
        offset+=line.len();
    }
    Err("即時 status 需使用獨立一行的簡單文字或雙引號字串；請在 Markdown 編輯其他寫法。".into())
}

const ROWS:&str=" FROM obsidian_base_live_columns b
    JOIN obsidian_base_file_values v ON v.collection_id=b.collection_id AND v.key=b.key
    JOIN vault_paths p ON p.note_id=v.note_id AND p.import_root=b.import_root
    JOIN notes root ON root.id=b.import_root AND root.trash_root IS NULL
    JOIN notes n ON n.id=v.note_id AND n.trash_root IS NULL
    WHERE b.collection_id=?1 AND b.key=?2";

pub(super) fn read_status(body:&str)->Result<String>{Ok(status_token(body)?.1)}

pub(super) fn display(db:&Connection,col:i64,key:&str,editable_only:bool)->Result<Option<HashMap<i64,String>>> {
    let binding:Option<Option<String>>=db.query_row("SELECT equals_text FROM obsidian_base_live_columns WHERE collection_id=?1 AND key=?2",params![col,key],|r|r.get(0)).optional()?;
    let Some(predicate)=binding else{return Ok(None)};
    if editable_only && predicate.is_some() { return Ok(Some(HashMap::new())); }
    let mut stmt=db.prepare(&format!("SELECT n.id,substr(CAST(n.body AS BLOB),1,?3){ROWS} LIMIT 1001"))?;
    let mut rows=stmt.query(params![col,key,(obsidian_base::FRONT_LIMIT+4) as i64])?;
    let mut values=HashMap::new();
    let mut budget=32*1024*1024usize;
    while let Some(row)=rows.next()? {
        let id:i64=row.get(0)?;
        let bytes=row.get_ref(1)?.as_blob()?;
        budget=budget.checked_sub(bytes.len()).ok_or("即時欄位超過 32 MiB 讀取上限。")?;
        // A bounded prefix can stop in the middle of a body character past the header.
        let prefix=match std::str::from_utf8(bytes) { Ok(s)=>s, Err(e) if e.error_len().is_none()=>std::str::from_utf8(&bytes[..e.valid_up_to()])?, Err(e)=>return Err(e.into()) };
        match read_status(prefix) {
            Ok(value) => {
                let shown=match &predicate {Some(expected)=>(value==*expected).to_string(),None=>value};
                values.insert(id,shown);
            }
            Err(_) if editable_only => {}
            Err(_) => {values.insert(id,"（status 無法讀取）".into());}
        }
        if values.len()>1000 {return Err("即時集合超過 1,000 筆。".into());}
    }
    Ok(Some(values))
}

impl Notebook {
    pub(crate) fn editable_status_rows(&self,col:i64,key:&str)->Result<HashMap<i64,String>> {
        Ok(display(&self.connection,col,key,true)?.unwrap_or_default())
    }

    pub(crate) fn live_status_keys(&self,col:i64)->Result<Vec<String>> {
        let mut stmt=self.connection.prepare("SELECT key FROM obsidian_base_live_columns WHERE collection_id=?1 AND equals_text IS NULL ORDER BY key")?;
        Ok(stmt.query_map([col],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }
    pub(crate) fn live_status_snapshot(&self,col:i64,key:&str,note:i64)->Result<(i64,String)> {
        let (version,body):(i64,String)=self.connection.query_row(&format!("SELECT n.version,n.body{ROWS} AND n.id=?3 AND b.equals_text IS NULL"),params![col,key,note],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or("這篇筆記沒有可寫入的來源狀態。")?;
        Ok((version,read_status(&body)?))
    }
    pub(crate) fn edit_live_status(&self,col:i64,key:&str,note:i64,expected:i64,value:&str)->Result<()> {
        if value.len()>1024 || value.chars().any(char::is_control) {return Err("status 需要 1024 位元組內的單行文字。".into());}
        let tx=Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        let (title,mut body):(String,String)=tx.query_row(&format!("SELECT n.title,n.body{ROWS} AND n.id=?3 AND n.version=?4 AND b.equals_text IS NULL"),params![col,key,note,expected],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or("來源或版本已變更，請重新開啟。")?;
        let (range,old)=status_token(&body)?;
        if old==value {return Ok(());}
        let replacement=serde_json::to_string(value)?;
        let mut marks=note_highlights::read_marks(&tx,note,&body)?;
        for mark in &mut marks {
            if mark.end<=range.start {continue}
            if mark.start<range.end {return Err("status 與現有高亮重疊，請先在筆記中調整。".into());}
            mark.start=mark.start-range.len()+replacement.len();
            mark.end=mark.end-range.len()+replacement.len();
        }
        body.replace_range(range,&replacement);
        if read_status(&body)?!=value {return Err("新值無法依原樣保存。".into());}
        save_in(&tx,note,expected,&title,&body)?;
        for mark in marks {tx.execute("INSERT INTO note_highlights(note_id,start_byte,end_byte) VALUES(?1,?2,?3)",params![note,mark.start as i64,mark.end as i64])?;}
        tx.commit()?;
        Ok(())
    }
}
