//! Materialize selected Vault Markdown IDs into native collections in one transaction.
use super::{Notebook, Result, attachments, collection_display::MAX_DISPLAY_COLUMNS,
    collections::{insert_collection, MAX_ITEMS}, insert, note_links::NATIVE_NOTE_PREFIX,
    obsidian_base::{self, Base, Layout, View}, properties::{self, key_checked}};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};


enum Column { Filename, Folder, Path, Extension, Formula(obsidian_base::FileFormula), Front(String) }

fn columns(base: &Base, view: &View) -> Result<Vec<(String,Column)>> {
    let ordered: Vec<&str> = if view.order.is_empty() { vec!["file.name"] }
        else { view.order.iter().map(String::as_str).collect() };
    if ordered.len() > MAX_DISPLAY_COLUMNS {
        return Err(format!("紙間每個檢視最多顯示 {MAX_DISPLAY_COLUMNS} 個額外欄位。").into());
    }
    let mut result = Vec::new();
    for key in ordered {
        let alias = base.display_names.get(key).map(String::as_str);
        let (name,column) = match key {
            "file.name" => (alias.unwrap_or(key), Column::Filename),
            "file.folder" => (alias.unwrap_or(key), Column::Folder),
            "file.path" => (alias.unwrap_or(key), Column::Path),
            "file.ext" => (alias.unwrap_or(key), Column::Extension),
            key if key.starts_with("formula.") => (alias.unwrap_or(&key[8..]),Column::Formula(base.formulas.get(key).ok_or("找不到這項公式。")?.clone())),
            key if key.starts_with("note.") => (alias.unwrap_or(&key[5..]), Column::Front(key[5..].to_owned())),
            _ => return Err(format!("無法從 Vault 原始資料確認欄位「{key}」。").into()),
        };
        if name.trim() != key_checked(name)? || name.trim()!=name
            || result.iter().any(|(existing,_)| existing==name) {
            return Err("欄位顯示名稱重複或包含無法保存的空白。".into());
        }
        result.push((name.to_owned(),column));
    }
    Ok(result)
}

fn snapshot_field(tx:&Transaction<'_>, collection:i64, note:i64, key:&str, value:&str) -> Result<()> {
    if value.len()>properties::MAX_VALUE_BYTES { return Err("Vault 路徑欄位超過筆記屬性上限。".into()); }
    tx.prepare_cached("INSERT INTO obsidian_base_file_values(collection_id,note_id,key,value) VALUES(?1,?2,?3,?4)")?
        .execute(params![collection,note,key,value])?;
    Ok(())
}

fn typed_prefix(tx:&Transaction<'_>,note:i64,budget:&mut usize)->Result<String>{
    let bytes:Vec<u8>=tx.prepare_cached("SELECT substr(CAST(body AS BLOB),1,?2) FROM notes WHERE id=?1 AND trash_root IS NULL")?
        .query_row(params![note,(obsidian_base::FRONT_LIMIT+4)as i64],|r|r.get(0))?;
    *budget=budget.checked_sub(bytes.len()).ok_or("型別欄位超過 32 MiB 讀取上限。")?;
    let size=std::str::from_utf8(&bytes).map_or_else(|e|e.valid_up_to(),|s|s.len());
    Ok(std::str::from_utf8(&bytes[..size])?.to_owned())
}

fn text_for_receipt(value:&str) -> String {
    let mut out=String::new();
    for ch in value.chars() {
        if ch.is_ascii_punctuation() { out.push('\\'); }
        out.push(ch);
    }
    out
}

impl Notebook {
    pub(crate) fn import_obsidian_base(&self, root:i64, filename:&str, raw:Vec<u8>)
        -> Result<(i64,Vec<(i64,String,i64,bool)>)> {
        self.import_obsidian_base_mode(root,filename,raw,false)
    }
    pub(crate) fn import_obsidian_base_mode(&self, root:i64, filename:&str, raw:Vec<u8>, live:bool)
        -> Result<(i64,Vec<(i64,String,i64,bool)>)> {
        self.import_obsidian_base_options(root,filename,raw,live,false)
    }
    pub(crate) fn import_obsidian_base_options(&self,root:i64,filename:&str,raw:Vec<u8>,live:bool,dynamic:bool)
        ->Result<(i64,Vec<(i64,String,i64,bool)>)>{
        crate::output::windows_output_name(filename.as_ref())?;
        let stem = filename.rsplit_once('.').filter(|(s,e)| !s.trim().is_empty() && e.eq_ignore_ascii_case("base"))
            .map(|(s,_)|s).ok_or("請選擇 .base 原檔。")?;
        if filename.encode_utf16().count()>255 || filename.chars().any(char::is_control)
            || !attachments::has_expected_signature("text",&raw) { return Err("Bases 檔名或文字格式無效。".into()); }
        let base = obsidian_base::parse(&raw)?;
        if !dynamic && base.views.iter().any(|v|!v.sorts.is_empty()){return Err("這份 Bases 含型別排序，請選擇動態模式。".into())}
        if dynamic && (!live || base.views.iter().any(|v|v.typed_filters.len()>8)){return Err("動態模式需連結來源，每個檢視最多 8 個條件。".into())}
        if live && !base.has_live_status() {return Err("這份 Bases 沒有連結的 status 或 status 等值公式。".into());}
        let tx=Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        let token:String=tx.query_row("SELECT i.token FROM notes n JOIN note_link_identities i ON i.note_id=n.id
            WHERE n.id=?1 AND n.trash_root IS NULL AND EXISTS(SELECT 1 FROM vault_paths WHERE import_root=n.id)",
            [root],|r|r.get(0)).optional()?.ok_or("請選擇仍在使用中的已匯入 Vault 根筆記。")?;
        let used:i64=tx.query_row("SELECT coalesce(sum(length(content)),0) FROM attachments",[],|r|r.get(0))?;
        if used>attachments::MAX_STORAGE_BYTES - base.raw.len() as i64 { return Err("工作區附件空間不足。".into()); }
        let mut report=format!("原始檔：{}\n\n來源 Vault：[[{NATIVE_NOTE_PREFIX}{root}:{token}|開啟來源]]\n\n這些集合使用匯入時的成員與檔案欄位。頁首欄位取自匯入此檢視當下，工作區內已儲存的 Markdown。可由集合選單開啟同名檢視。\n",text_for_receipt(filename));
        let mut created=Vec::new();
        let mut named_views=Vec::new();
        let normalized_handoff:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM heptabase_import_roots WHERE import_root=?1)",[root],|r|r.get(0))?;
        if (live || !base.types.is_empty()) && normalized_handoff {return Err("此 Markdown ZIP 來源可能曾移除首行，無法核對連結欄位與型別比較的位置。".into());}
        if dynamic{report.push_str("\n動態檢視會在開啟或重新整理時，以匯入時所選路徑的候選筆記重新篩選及排序；原檔改名或搬離該路徑即暫停入選，需重新匯入以採用新路徑。只讀取工作區已儲存的 Markdown。\n");}
        else if !base.types.is_empty(){report.push_str("\n布林與數值比較篩選會在匯入時決定成員；缺少與 null 皆不符合比較。\n");}
        if live {report.push_str("\n已連結的來源欄位使用工作區目前保存的 Markdown；編輯時只更新該筆記。\n");}
        let mut remaining_front_bytes=32*1024*1024usize;
        if base.display_names.keys().any(|key| !matches!(key.as_str(),"file.name" | "file.folder" | "file.path" | "file.ext") && !key.starts_with("note.") && !base.formulas.contains_key(key)) {
            return Err("只支援檔案與筆記頁首欄位的顯示名稱。".into());
        }
        for view in &base.views {
            let name=format!("{stem}／{}",view.name);
            let collection=insert_collection(&tx,&name)?;
            let cols=columns(&base,view)?;
            for (key, col) in &cols {
                tx.execute("INSERT INTO obsidian_base_file_columns(collection_id,key) VALUES(?1,?2)",params![collection,key])?;
                if live {
                    let binding=match col{
                        Column::Front(source)=>base.types.get(source).map(|kind|(source.as_str(),*kind,None)),
                        Column::Formula(expr)=>expr.typed().map(|t|(t.key.as_str(),t.kind,Some(t))),
                        _=>None
                    };
                    if let Some((source,kind,predicate))=binding{
                        tx.execute("INSERT INTO obsidian_base_typed_columns(collection_id,key,import_root,source_key,kind,op,operand) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                            params![collection,key,root,source,kind,predicate.map(|p|&p.op),predicate.map(|p|&p.operand)])?;
                    }
                }
                let predicate=match col {
                    Column::Front(key) if key=="status"=>Some(None),
                    Column::Formula(expr)=>expr.status_literal().map(Some),
                    _=>None
                };
                if live && let Some(equals)=predicate {
                    tx.execute("INSERT INTO obsidian_base_live_columns(collection_id,key,import_root,equals_text) VALUES(?1,?2,?3,?4)",params![collection,key,root,equals])?;
                }
            }
            if dynamic{super::obsidian_base_dynamic::register(&tx,collection,root,view,&base)?;}
            let mut members=Vec::new();
            {
                let mut stmt=tx.prepare_cached("SELECT p.note_id,p.relative_path FROM vault_paths p JOIN notes n ON n.id=p.note_id
                    WHERE p.import_root=?1 AND n.trash_root IS NULL ORDER BY p.relative_path")?;
                let mut rows=stmt.query([root])?;
                while let Some(row)=rows.next()? {
                    let path=row.get_ref(1)?.as_str()?;
                    if !view.matches_path(path) { continue; }
                    if !dynamic && !view.typed_filters.is_empty(){
                        let id:i64=row.get(0)?;
                        let prefix=typed_prefix(&tx,id,&mut remaining_front_bytes)?;
                        let mut include=true;
                        for test in &view.typed_filters{
                            if !super::obsidian_base_typed::filter(&prefix,test).map_err(|e|format!("「{path}」：{e}"))?{include=false;break}
                        }
                        if !include{continue;}
                    }
                    if members.len()>=MAX_ITEMS as usize { return Err("檢視超過 1,000 篇筆記。".into()); }
                    members.push((row.get::<_,i64>(0)?,path.to_owned()));
                }
            }
            let selected: std::collections::BTreeSet<_> = cols.iter().filter_map(|(_,col)| match col {
                Column::Front(key) if !base.types.contains_key(key) && (!live || key != "status") => Some(key.as_str()),
                Column::Formula(expr) if !live && expr.status_literal().is_some() => Some("status"),
                _ => None,
            }).collect();
            for (note,path) in &members {
                let fields=if !selected.is_empty() {
                    let prefix:Vec<u8>=tx.prepare_cached("SELECT substr(CAST(body AS BLOB),1,?2) FROM notes WHERE id=?1 AND trash_root IS NULL")?
                        .query_row(params![note,(obsidian_base::FRONT_LIMIT+4) as i64],|r|r.get(0))?;
                    let start=prefix.strip_prefix(&[0xef,0xbb,0xbf]).unwrap_or(&prefix);
                    if normalized_handoff && (start.starts_with(b"---\n") || start.starts_with(b"---\r\n") || start==b"---") {
                        return Err(format!("「{path}」的 Markdown ZIP 可能已移除首行標題，無法核對頁首位置。").into());
                    }
                    remaining_front_bytes=remaining_front_bytes.checked_sub(prefix.len()).ok_or("頁首資料讀取量超過 32 MiB，請縮小檢視範圍。")?;
                    if !live && cols.iter().any(|(_,c)|matches!(c,Column::Formula(e) if e.status_literal().is_some())) {
                        let valid=std::str::from_utf8(&prefix).map_or_else(|e|e.valid_up_to(),|s|s.len());
                        super::obsidian_base_live::read_status(std::str::from_utf8(&prefix[..valid])?).map_err(|e|format!("「{path}」：{e}"))?;
                    }
                    obsidian_base::front_fields(&prefix,&selected).map_err(|err|format!("「{path}」：{err}"))?
                }else{Default::default()};
                let (folder,file)=path.rsplit_once('/').unwrap_or(("",path));
                tx.execute("INSERT INTO collection_notes(collection_id,note_id,status) VALUES(?1,?2,'已匯入')",params![collection,note])?;
                let typed_source=if !live && cols.iter().any(|(_,c)|match c{Column::Front(k)=>base.types.contains_key(k),Column::Formula(e)=>e.typed().is_some(),_=>false}){
                    Some(typed_prefix(&tx,*note,&mut remaining_front_bytes)?)
                }else{None};
                if dynamic{
                    tx.execute("INSERT INTO obsidian_base_candidates(collection_id,note_id,relative_path) VALUES(?1,?2,?3)",params![collection,note,path])?;
                }
                for (label,col) in &cols {
                    let ty=match col{
                        Column::Front(k)=>base.types.get(k).map(|kind|(k.as_str(),*kind,None)),
                        Column::Formula(e)=>e.typed().map(|t|(t.key.as_str(),t.kind,Some(t))),
                        _=>None
                    };
                    if let Some((source,kind,test))=ty{
                        let value=if let Some(prefix)=&typed_source{super::obsidian_base_typed::snapshot(prefix,source,kind,test).map_err(|e|format!("「{path}」：{e}"))?}else{String::new()};
                        snapshot_field(&tx,collection,*note,label,&value)?;
                        continue;
                    }
                    let value=match col {
                        // These rows fix the membership of the binding. The current value is read from the note.
                        Column::Front(key) if live && key == "status" => "",
                        Column::Formula(expr) if live && expr.status_literal().is_some() => "",
                        Column::Formula(expr) if expr.status_literal().is_some() => if fields.get("status").map(String::as_str)==expr.status_literal() {"true"} else {"false"},
                        Column::Formula(expr) => expr.display(path,folder,file),
                        Column::Filename => file,
                        Column::Folder => folder,
                        Column::Path => path,
                        Column::Extension => file.rsplit_once('.').map_or("", |(_, ext)| ext),
                        Column::Front(key) => match fields.get(key) { Some(value) => value, None => continue },
                    };
                    snapshot_field(&tx,collection,*note,label,value)?;
                }
            }
            let list=view.layout==Layout::List;
            tx.execute("INSERT INTO collection_views(collection_id,name,layout,status,keyword,list_mode)
                VALUES(?1,?2,'table','','',?3)",params![collection,&view.name,list])?;
            named_views.push((view.name.clone(),tx.last_insert_rowid()));
            if let Some(group)=&view.group {
                let source=group.strip_prefix("note.").ok_or("無效的群組來源。")?;
                let kind=base.types.get(source).ok_or("無法核對群組欄位型別。")?;
                let label=base.display_names.get(group).map_or(source,String::as_str);
                let view_id=tx.last_insert_rowid();
                tx.execute("UPDATE collection_views SET group_key=?2 WHERE id=?1",params![view_id,label])?;
                tx.execute("INSERT INTO obsidian_base_typed_groups(view_id,label,kind) VALUES(?1,?2,?3)",params![view_id,label,kind])?;
            }
            let labels:Vec<_>=cols.iter().map(|(key,_)|key).collect();
            tx.execute("INSERT INTO collection_display_columns(collection_id,columns_json) VALUES(?1,?2)",
                params![collection,serde_json::to_string(&labels)?])?;
            let count=if dynamic{super::obsidian_base_dynamic::rows(&tx,collection)?.ok_or("找不到動態候選")?.len()}else{members.len()};
            report.push_str(&format!("\n- 集合 #{}　{}（匯入時 {} 篇）",collection,text_for_receipt(&name),count));
            created.push((collection,name,count as i64,list));
        }
        let receipt=insert(&tx,Some(root),&format!("Bases 來源：{filename}"),&report)?;
        tx.execute("INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,?2,'text',?3)",
            params![receipt,filename,base.raw])?;
        let attachment=tx.last_insert_rowid();
        tx.execute("INSERT INTO base_card_receipts(receipt,import_root,attachment,filename) VALUES(?1,?2,?3,?4)",
            params![receipt,root,attachment,filename])?;
        for (name,view) in named_views {
            tx.execute("INSERT INTO base_card_views(receipt,name,view_id) VALUES(?1,?2,?3)",params![receipt,name,view])?;
        }
        tx.commit()?;
        Ok((receipt,created))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    fn page(dir:&str)->ImportedPage {
        ImportedPage{parts:vec![dir.into(),"鏡頭.md".into()],title:"編輯後標題".into(),body:"內容".into()}
    }
    #[test]
    fn exact_file_path_filter_selects_distinct_case_sensitive_vault_members() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let files=["Default-307.md","Alpha-616.md","Bravo-926.md","alpha-616.md"];
        let root=db.import_vault("Shipping",files.iter().map(|f|ImportedPage{parts:vec![f.to_string()],title:"同名".into(),body:"Proof".into()}).collect())?;
        let mut raw="views:\n".to_string();
        for file in &files[..3]{raw+=&format!("  - type: table\n    name: {file}\n    filters: 'file.path == \"{file}\"'\n    order: [file.name]\n");}
        let (_,views)=db.import_obsidian_base(root,"Shipping.base",raw.into_bytes())?;
        for ((col,_,count,_),file) in views.iter().zip(files){assert_eq!(*count,1);assert_eq!(db.collection_display_values(*col,"file.name")?.into_values().next().unwrap(),file);}
        for bad in ["../Alpha.md","CON.md","A//B.md"]{
            let input=format!("views: [{{type: table, name: X, filters: 'file.path == \"{bad}\"'}}]");
            assert!(obsidian_base::parse(input.as_bytes()).is_err());
        }
        Ok(())
    }

    #[test]
    fn dynamic_saved_rules_readmit_excluded_notes_and_restore_sort_without_writing() -> Result<()> {
        let dir=std::env::temp_dir().join(format!("paperroom-dynamic-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        std::fs::create_dir(&dir)?;
        let path=dir.join("workspace.sqlite");
        let mut db=Notebook::open(&path,false)?;
        fn content(approved:&str,cost:&str,priority:&str)->String{format!("---\napproved: {approved}\ncost: {cost}\npriority: {priority}\n---\n正文")}
        let mut pages=Vec::new();
        for (name,approved,cost,priority) in [("C","true","10","false"),("B","true","2.0","true"),("A","false","2.00","true"),("G","false","2","false"),("D","true","2","false"),("E","true","null","false"),("F","true","[bad]","false")]{
            let mut p=page(name);p.body=content(approved,cost,priority);pages.push(p);
        }
        let root=db.import_vault("來源庫",pages)?;
        let mut unrelated=page("A");unrelated.body=content("true","1","true");db.import_vault("另庫",vec![unrelated])?;
        let raw=r#"filters: 'note.approved == true'
formulas: {more: 'note.cost >= 0', priorityFlag: 'note.priority == true'}
views:
  - type: table
    name: 動態
    order: [file.path, approved, cost, priority]
    sort: [{property: note.cost, direction: ASC}, {property: note.priority, direction: DESC}]
"#;
        assert!(db.import_obsidian_base_mode(root,"unsupported.base",raw.as_bytes().to_vec(),true).is_err());
        let (receipt,views)=db.import_obsidian_base_options(root,"shipping.base",raw.as_bytes().to_vec(),true,true)?;
        assert_eq!(views[0].2,5);
        let col=views[0].0;
        let rows=|db:&Notebook|->Result<Vec<String>>{
            let view=db.collection_views(col)?.remove(0);let (lanes,fields)=db.collection_lanes_and_display(col,&view)?;
            let ids=lanes.into_iter().flat_map(|(_,rows)|rows.into_iter().map(|r|r.id)).collect::<Vec<_>>();
            let fields=fields.unwrap();assert_eq!(fields[0].1.len(),ids.len());
            Ok(ids.iter().map(|id|fields[0].1[id].split('/').next().unwrap().to_string()).collect())
        };
        assert_eq!(rows(&db)?,["B","D","C","E","F"]);
        let id=|prefix:&str|->Result<i64>{Ok(db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",params![root,format!("{prefix}/鏡頭.md")],|r|r.get(0))?)};
        let a=id("A")?;let b=id("B")?;
        assert!(!db.typed_base_rows(col,"cost")?.contains_key(&a));
        let before=db.connection.total_changes();
        assert!(db.add_collection_entry(col,super::super::collections::CollectionEntry::New("不應建立"),"新增",None).is_err());
        assert!(db.add_to_collection(col,root,"新增").is_err());
        assert!(db.remove_from_collection(col,b,"已匯入").is_err());
        assert!(db.move_collection_note(col,b,"已匯入","移動").is_err());
        assert_eq!(db.connection.total_changes(),before);
        db.connection.execute("UPDATE vault_paths SET relative_path='B/重新命名.md' WHERE note_id=?1",[b])?;
        assert!(!db.collection_rows(col)?.iter().any(|(row,_)|row.id==b));
        db.connection.execute("UPDATE vault_paths SET relative_path='B/鏡頭.md' WHERE note_id=?1",[b])?;
        let (stale,_)=db.typed_base_current(col,"cost",b)?;
        let writer=Notebook::open(&path,false)?;
        let edit=|db:&Notebook,note:i64,approved:&str|->Result<()>{
            let n=db.note(note)?.unwrap();db.save(note,n.version,&n.row.title,&content(approved,"2.00","true"))?;Ok(())
        };
        edit(&writer,a,"true")?;
        assert_eq!(rows(&db)?,["A","B","D","C","E","F"]);
        edit(&writer,b,"false")?;
        assert!(db.edit_typed_base(col,"cost",b,stale,"9").is_err());
        assert_eq!(rows(&db)?,["A","D","C","E","F"]);
        let history=db.note_revisions(a)?.len();
        let writes=db.connection.total_changes();assert_eq!(rows(&db)?,["A","D","C","E","F"]);
        assert_eq!(writer.note_revisions(a)?.len(),history); assert_eq!(db.connection.total_changes(),writes);
        db.trash(a)?;assert_eq!(rows(&db)?,["D","C","E","F"]);db.restore(a)?;
        let backup=dir.join("restore.sqlite");db.backup(&backup)?;
        drop(writer);drop(db);let restored=Notebook::open(&backup,false)?;
        assert_eq!(rows(&restored)?,["A","D","C","E","F"]);edit(&restored,b,"true")?;
        assert_eq!(rows(&restored)?,["A","B","D","C","E","F"]);
        assert_eq!(restored.connection.query_row("SELECT content FROM attachments WHERE note_id=?1",[receipt],|r|r.get::<_,Vec<u8>>(0))?,raw.as_bytes());
        drop(restored);std::fs::remove_dir_all(dir)?;Ok(())
    }
    #[test]
    fn typed_group_uses_numeric_order_and_recomputes_after_save_and_backup() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let mut pages=Vec::new();
        for (folder,value) in [("A","10"),("B","2.0"),("C","2.00"),("D","-1"),("E","null"),("F","")]{
            let mut p=page(folder);p.body=if value.is_empty(){"沒有頁首".into()}else{format!("---\ncost: {value}\n---\n保留")};pages.push(p);
        }
        let root=db.import_vault("分組",pages)?;
        let raw=r#"formulas: {over: 'note.cost > 2'}
properties: {cost: {displayName: 成本}}
views: [{type: table, name: 費用, order: [cost,formula.over], groupBy: {property: note.cost, direction: ASC}}]"#;
        let (_,views)=db.import_obsidian_base_mode(root,"group.base",raw.as_bytes().to_vec(),true)?;
        let col=views[0].0;
        let layout=|db:&Notebook|->Result<Vec<(String,usize)>>{
            let view=db.collection_views(col)?.into_iter().find(|v|v.name=="費用").unwrap();
            let (lanes,values)=db.collection_lanes_and_display(col,&view)?;
            assert_eq!(values.unwrap().len(),2);
            Ok(lanes.into_iter().map(|(group,rows)|(group.0.unwrap(),rows.len())).collect())
        };
        let want=|items:&[(&str,usize)]|items.iter().map(|(s,n)|(s.to_string(),*n)).collect::<Vec<_>>();
        assert_eq!(layout(&db)?,want(&[("-1",1),("2",2),("10",1),("（缺少）",1),("null",1)]));
        let id=db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path LIKE 'A/%'",[root],|r|r.get(0))?;
        let (ver,_)=db.typed_base_current(col,"成本",id)?;
        db.edit_typed_base(col,"成本",id,ver,"2")?;
        let expected=want(&[("-1",1),("2",3),("（缺少）",1),("null",1)]);
        assert_eq!(layout(&db)?,expected);
        let view=db.collection_views(col)?.remove(0);
        let two=super::super::collection_group_folds::GroupValue(Some("2".into()));
        let axes=db.collection_group_folds(&view)?.unwrap();
        db.set_collection_group_fold(col,&view,&axes,Some((&two,true)))?;
        db.compare_set_property_type("成本",None,Some(super::super::typed_properties::PropertyType::Date))?;
        assert!(db.collection_group_folds(&view)?.unwrap().contains(&two));
        let duplicate_id=db.duplicate_collection_view(col,&view,"分組副本")?;
        let duplicate=db.collection_views(col)?.into_iter().find(|v|v.id==duplicate_id).unwrap();
        assert_eq!(db.presented_collection_lanes(col,Some(&duplicate))?.into_iter().map(|(g,r)|(g.0.unwrap(),r.len())).collect::<Vec<_>>(),expected);
        assert!(!db.collection_group_folds(&duplicate)?.unwrap().contains(&two));
        let destination=db.create_collection("無對應來源")?;
        assert!(db.copy_collection_view_to(col,&view,destination,"不可複製").is_err());
        assert!(db.collection_views(destination)?.is_empty());
        let dir=std::env::temp_dir().join(format!("paperroom-bases-groups-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        std::fs::create_dir(&dir)?;
        let file=dir.join("backup.sqlite");db.backup(&file)?;
        let reopened=Notebook::open(&file,false)?;
        assert_eq!(layout(&reopened)?,expected);
        assert!(reopened.collection_group_folds(&view)?.unwrap().contains(&two));
        assert_eq!(reopened.presented_collection_lanes(col,Some(&duplicate))?.into_iter().map(|(g,r)|(g.0.unwrap(),r.len())).collect::<Vec<_>>(),expected);
        assert_eq!(reopened.typed_base_current(col,"成本",id)?.1,"2");
        drop(reopened);std::fs::remove_dir_all(dir)?;
        assert!(db.import_obsidian_base_mode(root,"bad.base",raw.replace("direction: ASC","direction: DESC").into_bytes(),true).is_err());
        for invalid in ["note.true == false","note.null == 0","note.0 >= 1"]{
            assert!(db.import_obsidian_base_mode(root,"invalid.base",raw.replace("note.cost > 2",invalid).into_bytes(),true).is_err());
        }
        Ok(())
    }
    #[test]
    fn typed_booleans_and_exact_decimals_share_aliases_with_null_missing_and_named_filters() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let mut pages=Vec::new();
        for (folder,body) in [("A","\u{feff}---\r\napproved: false # 保留註解\r\ncost: 0  # 價格\r\n---\r\n後文🙂"),("B","---\napproved: # 未決\ncost: null\n---\nB"),("C","原始正文"),("D","---\napproved: true\ncost: 2.500001\n---\nD")] {
            let mut p=page(folder);p.body=body.into();pages.push(p);
        }
        let root=db.import_vault("專案",pages)?;
        let mut other=page("A");other.body="---\napproved: false\ncost: 100\n---".into();
        db.import_vault("另庫",vec![other])?;
        let raw=r#"properties: {approved: {displayName: 已核准}, cost: {displayName: 價格}}
formulas: {yes: 'note.approved == true', over: 'note.cost >= 2.5'}
views:
  - {type: table, name: 全部, order: [approved, cost, formula.yes, formula.over]}
  - {type: list, name: 初始篩選, filters: 'note.cost >= 2.5', order: [cost]}
"#;
        let (_,views)=db.import_obsidian_base_mode(root,"A.base",raw.as_bytes().to_vec(),true)?;
        let (col,filtered)=(views[0].0,views[1].0);
        assert_eq!((views[0].2,views[1].2),(4,1));
        let (_,bviews)=db.import_obsidian_base_mode(root,"B.base",raw.replace("價格","成本").into_bytes(),true)?;
        let (_,snap)=db.import_obsidian_base_mode(root,"S.base",raw.as_bytes().to_vec(),false)?;
        let ids:Vec<i64>=db.connection.prepare("SELECT note_id FROM vault_paths WHERE import_root=?1 ORDER BY relative_path")?
            .query_map([root],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let (a,b,c,d)=(ids[0],ids[1],ids[2],ids[3]);
        for (id,approved,cost,yes,over) in [(a,"false","0","false","false"),(b,"null","null","null","null"),(c,"（缺少）","（缺少）","（缺少）","（缺少）"),(d,"true","2.500001","true","true")] {
            for (label,want) in [("已核准",approved),("價格",cost),("yes",yes),("over",over)] {
                assert_eq!(&db.collection_display_values(col,label)?[&id],want);
            }
        }
        db.set_property(a,"價格",Some("原生值"))?;
        let first=db.note(a)?.unwrap();
        let start=first.body.find("後文").unwrap();
        db.connection.execute("INSERT INTO note_highlights(note_id,start_byte,end_byte) VALUES(?1,?2,?3)",params![a,start as i64,first.body.len() as i64])?;
        db.edit_typed_base(col,"價格",a,first.version,"2.499999")?;
        assert!(db.edit_typed_base(col,"價格",a,first.version,"7").is_err());
        let current=db.note(a)?.unwrap();
        assert_eq!(current.body,first.body.replace("cost: 0","cost: 2.499999"));
        let marks=db.note_highlights(a,&current.body)?;
        assert_eq!(&current.body[marks[0].clone()],"後文🙂");
        assert_eq!(db.note_revision(a,db.note_revisions(a)?[0].id)?.1,first.body);
        assert_eq!(&db.collection_display_values(bviews[0].0,"成本")?[&a],"2.499999");
        assert_eq!(&db.collection_display_values(bviews[0].0,"over")?[&a],"false");
        db.edit_typed_base(bviews[0].0,"成本",a,current.version,"2.500000")?;
        assert_eq!(&db.collection_display_values(col,"over")?[&a],"true");
        assert!(!db.collection_display_values(filtered,"價格")?.contains_key(&a));
        assert_eq!(&db.collection_display_values(snap[0].0,"價格")?[&a],"0");
        assert_eq!(db.properties(a)?,vec![("價格".into(),"原生值".into())]);
        for (id,key,val) in [(b,"已核准","false"),(b,"價格","0"),(c,"已核准","true"),(c,"價格","-2.125"),(c,"已核准","null")] {
            let (v,_)=db.typed_base_current(col,key,id)?;db.edit_typed_base(col,key,id,v,val)?;
            assert_eq!(&db.collection_display_values(col,key)?[&id],val);
        }
        assert_eq!(db.note(c)?.unwrap().body,"---\ncost: -2.125\napproved: null\n---\n原始正文");
        let (v,_)=db.typed_base_current(col,"價格",a)?;
        for invalid in ["1e2","1.2345678","1000000000000000","true","NaN","01",""] {
            assert!(db.edit_typed_base(col,"價格",a,v,invalid).is_err());
        }
        assert_eq!(db.note(a)?.unwrap().version,v);
        let malformed="---\ncost: \"2.5\"\napproved: [false]\n---";
        db.save(a,v,"仍是同一篇",malformed)?;
        assert_eq!(&db.collection_display_values(col,"over")?[&a],"（型別或格式不符）");
        assert!(!db.typed_base_rows(col,"價格")?.contains_key(&a));
        assert_eq!(&db.collection_display_values(col,"價格")?[&b],"0");
        assert!(db.import_obsidian_base_mode(root,"conflict.base",raw.replace("note.cost >= 2.5","note.approved >= 2.5").into_bytes(),true).is_err());
        Ok(())
    }
    #[test]
    fn bad_live_status_is_local_to_its_row_and_recovers_after_normal_edit() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let mut a=page("甲");let mut b=page("乙");
        a.body="---\nstatus: 完成\n---".into();b.body=a.body.clone();
        let root=db.import_vault("來源",vec![a,b])?;
        let raw="formulas: {ok: 'status == \"完成\"'}\nviews: [{type: table, name: 所有, order: [status, formula.ok]}]";
        let (_,views)=db.import_obsidian_base_mode(root,"rows.base",raw.as_bytes().to_vec(),true)?;
        let col=views[0].0;
        let ids:Vec<i64>=db.connection.prepare("SELECT note_id FROM vault_paths WHERE import_root=?1 ORDER BY relative_path")?
            .query_map([root],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let (bad,good)=(ids[0],ids[1]);
        for broken in ["取消頁首","---\nstatus: [a,b]\n---","---\nstatus: old\nstatus: duplicate\n---"] {
            let n=db.note(bad)?.unwrap();db.save(bad,n.version,&n.row.title,broken)?;
            let shown=db.collection_display_snapshot(col)?.unwrap();
            assert_eq!(&shown[0].1[&bad],"（status 無法讀取）");
            assert_eq!(&shown[1].1[&bad],"（status 無法讀取）");
            assert_eq!(&shown[1].1[&good],"true");
            assert_eq!(db.editable_status_rows(col,"status")?.keys().copied().collect::<Vec<_>>(),vec![good]);
            assert!(db.live_status_snapshot(col,"status",bad).is_err());
        }
        let (v,_)=db.live_status_snapshot(col,"status",good)?;
        db.edit_live_status(col,"status",good,v,"待辦")?;
        assert_eq!(&db.collection_display_values(col,"ok")?[&good],"false");
        let n=db.note(bad)?.unwrap();db.save(bad,n.version,&n.row.title,"---\nstatus: 完成\n---")?;
        assert_eq!(db.editable_status_rows(col,"status")?.len(),2);
        assert_eq!(&db.collection_display_values(col,"ok")?[&bad],"true");
        Ok(())
    }
    #[test]
    fn writable_status_aliases_share_canonical_source_and_recalculate_only_when_opened() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let original="\u{feff}---\r\nstatus: 待辦\r\ntags: [test]\r\n---\r\n正文🙂提示";
        let mut a=page("甲");a.body=original.into();
        let mut b=page("乙"); b.body="---\nstatus: 保留\n---\n乙".into();
        let root=db.import_vault("測試庫",vec![a,b])?;
        let id_for=|path:&str|->rusqlite::Result<i64>{db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",params![root,path],|r|r.get(0))};
        let a=id_for("甲/鏡頭.md")?;let b=id_for("乙/鏡頭.md")?;
        db.set_property(a,"進度",Some("原生值"))?;
        let start=original.find("提示").unwrap();
        db.connection.execute("INSERT INTO note_highlights(note_id,start_byte,end_byte) VALUES(?1,?2,?3)",params![a,start as i64,original.len() as i64])?;
        let mut ids=Vec::new();
        for (filename,alias,live) in [("A.base","進度",true),("B.base","狀態",true),("S.base","快照",false)] {
            let raw=format!("properties: {{status: {{displayName: {alias}}}}}\nformulas: {{done: 'note.status == \"完成\"'}}\nviews: [{{type: table, name: 表, filters: 'file.folder == \"甲\"', order: [status, formula.done]}}]");
            let (_,saved)=db.import_obsidian_base_mode(root,filename,raw.into_bytes(),live)?;
            ids.push(saved[0].0);
        }
        let (v,initial)=db.live_status_snapshot(ids[0],"進度",a)?;
        assert_eq!(initial,"待辦");
        assert!(db.edit_live_status(ids[0],"進度",b,v,"不可寫").is_err());
        db.edit_live_status(ids[0],"進度",a,v,"完成")?;
        assert!(db.edit_live_status(ids[1],"狀態",a,v,"過時").is_err());
        let values=db.collection_display_snapshot(ids[1])?.unwrap();
        assert_eq!(values[0].1.get(&a).unwrap(),"完成");
        assert_eq!(values[1].1.get(&a).unwrap(),"true");
        let body=db.note(a)?.unwrap().body;
        assert_eq!(body,original.replacen("status: 待辦","status: \"完成\"",1));
        let marks=db.note_highlights(a,&body)?;
        assert_eq!(marks.len(),1);assert_eq!(&body[marks[0].clone()],"提示");
        let revision=db.note_revisions(a)?;
        assert_eq!(db.note_revision(a,revision[0].id)?.1,original);
        let (next,_)=db.live_status_snapshot(ids[1],"狀態",a)?;
        db.edit_live_status(ids[1],"狀態",a,next,"規劃中")?;
        assert_eq!(db.live_status_snapshot(ids[0],"進度",a)?.1,"規劃中");
        assert_eq!(db.collection_display_values(ids[0],"done")?.get(&a).unwrap(),"false");
        assert_eq!(db.collection_display_values(ids[2],"快照")?.get(&a).unwrap(),"待辦");
        assert!(db.live_status_snapshot(ids[2],"快照",a).is_err());
        assert_eq!(db.note(b)?.unwrap().body,"---\nstatus: 保留\n---\n乙");
        assert_eq!(db.properties(a)?,vec![("進度".into(),"原生值".into())]);
        let saved=db.note(a)?.unwrap();
        db.save(a,saved.version,&saved.row.title,&saved.body.replace("規劃中","完成"))?;
        assert_eq!(db.collection_display_values(ids[0],"done")?.get(&a).unwrap(),"true");
        let dir=std::env::temp_dir().join(format!("paperroom-bases-live-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        std::fs::create_dir(&dir)?;
        let file=dir.join("backup.sqlite");db.backup(&file)?;
        let restored=Notebook::open(&file,false)?;
        assert_eq!(restored.live_status_snapshot(ids[1],"狀態",a)?.1,"完成");
        assert_eq!(restored.collection_display_values(ids[0],"done")?.get(&a).unwrap(),"true");
        drop(restored);std::fs::remove_dir_all(dir)?;
        db.remove_from_collection(ids[0],a,"已匯入")?;
        db.add_to_collection(ids[0],a,"重新加入")?;
        assert!(db.live_status_snapshot(ids[0],"進度",a).is_err());
        Ok(())
    }
    #[test]
    fn file_formulas_are_snapshot_columns_for_exact_vault_members() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let root=db.import_vault("來源",vec![page("甲"),page("乙")])?;
        db.import_vault("另一個",vec![page("甲")])?;
        let raw=r#"formulas:
  folder: 'file.folder.startsWith("甲")'
  shot: 'file.path.contains("鏡頭")'
  caps: 'file.ext.endsWith("MD")'
properties: {formula.folder: {displayName: 選取}}
views: [{type: table, name: Formula, order: [formula.folder, formula.shot, formula.caps]}]
"#;
        let (receipt,views)=db.import_obsidian_base(root,"checks.base",raw.as_bytes().to_vec())?;
        let col=views[0].0;
        assert_eq!(views[0].2,2);
        for (path,expected) in [("甲/鏡頭.md","true"),("乙/鏡頭.md","false")] {
            let id:i64=db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",params![root,path],|r|r.get(0))?;
            db.set_property(id,"選取",Some("手動值"))?;
            for (key, value) in [("選取",expected),("shot","true"),("caps","false")] {
                assert_eq!(db.collection_display_values(col,key)?.get(&id).map(String::as_str),Some(value));
            }
        }
        assert_eq!(db.connection.query_row("SELECT content FROM attachments WHERE note_id=?1",[receipt],|r|r.get::<_,Vec<u8>>(0))?,raw.as_bytes());
        let before=db.collections()?;
        for bad in [raw.replace("file.path.contains", "file.path.unknown"),raw.replace("file.path.contains", "file.name.contains"),raw.replace("order: [formula.folder","order: [formula.missing"),raw.replace("file.ext.endsWith(\"MD\")","formula.caps"),raw.replace("file.ext.endsWith(\"MD\")","file.ext.endsWith(\"md\") + 1")] {
            assert!(db.import_obsidian_base(root,"bad.base",bad.into_bytes()).is_err());
            assert_eq!(db.collections()?,before);
        }
        Ok(())
    }
    #[test]
    fn materializes_scoped_ids_and_keeps_original_blob() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let root=db.import_vault("甲庫",vec![page("甲"),page("乙")])?;
        db.import_vault("另庫",vec![page("甲")])?;
        let raw=b"views:\r\n  - type: list\r\n    name: Shots\r\n    filters: 'file.folder == \"A\"'\r\n    order: [file.folder, file.name, file.path, file.ext]\r\n";
        let raw=String::from_utf8(raw.to_vec())?.replace("\"A\"","\"甲\"").into_bytes();
        let (receipt,views)=db.import_obsidian_base(root,"研究.base",raw.clone())?;
        let col=views[0].0;
        assert_eq!((views[0].2,views[0].3),(1,true));
        assert_eq!(db.collection_display_columns(col)?.unwrap(),["file.folder","file.name","file.path","file.ext"]);
        let (note,path):(i64,String)=db.connection.query_row("SELECT c.note_id,p.relative_path FROM collection_notes c JOIN vault_paths p ON p.note_id=c.note_id WHERE c.collection_id=?1 AND p.import_root=?2",params![col,root],|r|Ok((r.get(0)?,r.get(1)?)))?;
        assert_eq!(path,"甲/鏡頭.md");
        assert_eq!(db.connection.query_row("SELECT value FROM obsidian_base_file_values WHERE note_id=?1 AND collection_id=?2 AND key='file.name'",params![note,col],|r|r.get::<_,String>(0))?,"鏡頭.md");
        assert_eq!(db.collection_display_values(col, "file.path")?.get(&note).map(String::as_str),Some("甲/鏡頭.md"));
        assert_eq!(db.collection_display_values(col, "file.ext")?.get(&note).map(String::as_str),Some("md"));
        assert_eq!(db.connection.query_row("SELECT content FROM attachments WHERE note_id=?1 AND name='研究.base'",[receipt],|r|r.get::<_,Vec<u8>>(0))?,raw);
        Ok(())
    }
    #[test]
    fn file_alias_does_not_fall_back_to_global_value_after_membership_changes() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let root=db.import_vault("庫",vec![page("甲")])?;
        let note:i64=db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1",[root],|r|r.get(0))?;
        db.set_property(note,"alias",Some("私人屬性"))?;
        let raw=br#"properties:
  file.folder: {displayName: alias}
views:
  - {type: table, name: Included, order: [file.folder]}
  - {type: table, name: Empty, order: [file.folder], filters: 'file.folder == "none"'}
"#;
        let (_,views)=db.import_obsidian_base(root,"test.base",raw.to_vec())?;
        let included=views[0].0;
        let empty=views[1].0;
        assert_eq!(views[1].2,0);
        assert_eq!(db.collection_display_values(included,"alias")?.get(&note).map(String::as_str),Some("甲"));
        db.remove_from_collection(included,note,"已匯入")?;
        for collection in [included,empty] {
            db.add_to_collection(collection,note,"待辦")?;
            assert!(!db.collection_display_values(collection,"alias")?.contains_key(&note));
        }
        let sibling=db.create_collection("一般集合")?;
        db.add_to_collection(sibling,note,"待辦")?;
        assert_eq!(db.collection_display_values(sibling,"alias")?.get(&note).map(String::as_str),Some("私人屬性"));
        Ok(())
    }
    #[test]
    fn imported_alias_snapshots_do_not_overwrite_native_properties_or_namesakes() -> Result<()> {
        use rusqlite::params;
        let mut db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let mut a = page("甲");
        a.title = "鏡頭".into();
        a.body = "---\nstatus: 待辦\n---\n甲正文".into();
        let mut other = page("乙");
        other.title = "鏡頭".into();
        other.body = "---\nstatus: 乙保留\n---\n乙正文".into();
        let root = db.import_vault("同名測試庫", vec![a, other])?;
        let find = |relative: &str| db.connection.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            params![root, relative], |r| r.get::<_,i64>(0),
        );
        let (a, other) = (find("甲/鏡頭.md")?, find("乙/鏡頭.md")?);
        db.set_property(a, "進度", Some("原生值"))?;
        let mut collections = Vec::new();
        for (name, selector, alias) in [("A.base","note.status","進度"),("B.base","status","狀態")] {
            let raw = format!("properties:\n  {selector}: {{displayName: {alias}}}\nviews:\n  - type: table\n    name: 表格\n    filters: 'file.folder == \"甲\"'\n    order: [{selector}]\n");
            let (_, saved) = db.import_obsidian_base(root, name, raw.into_bytes())?;
            collections.push((saved[0].0, alias));
        }
        let note = db.note(a)?.unwrap();
        db.save(a, note.version, &note.row.title, "---\nstatus: 完成\n---\n甲正文")?;
        for (id, alias) in collections {
            let projection = db.collection_display_values(id, alias)?;
            assert_eq!(projection.len(), 1);
            assert_eq!(projection.get(&a).map(String::as_str), Some("待辦"));
        }
        assert_eq!(db.properties(a)?,vec![("進度".into(),"原生值".into())]);
        assert_eq!(db.note(other)?.unwrap().body, "---\nstatus: 乙保留\n---\n乙正文");
        Ok(())
    }

    #[test]
    fn scalar_frontmatter_uses_saved_markdown_and_preserves_missing_false_zero() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let mut source=page("甲");
        let updated="\u{feff}---\r\nflag: false\r\nn: 0\r\nref: '[[鏡頭]]'\r\nunset: null\r\ntags: [one, two]\r\nextra: {meta: [a, b]}\r\n---\r\n原文";
        source.body="---\nflag: true\nn: 3\n---\n初稿".into();
        let root=db.import_vault("庫",vec![source])?;
        let note:i64=db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1",[root],|r|r.get(0))?;
        let version:i64=db.connection.query_row("SELECT version FROM notes WHERE id=?1",[note],|r|r.get(0))?;
        db.save(note,version,"已編輯",updated)?;
        db.set_property(note,"缺值",Some("私人屬性"))?;
        let raw="properties: {unset: {displayName: 缺值}}\nviews: [{type: table, name: A, order: [flag, note.n, ref, unset]}]";
        let (_,views)=db.import_obsidian_base(root,"scalar.base",raw.as_bytes().to_vec())?;
        let collection=views[0].0;
        for (key,expected) in [("flag",Some("false")),("n",Some("0")),("ref",Some("[[鏡頭]]")),("缺值",None)] {
            assert_eq!(db.collection_display_values(collection,key)?.get(&note).map(String::as_str),expected);
        }
        for invalid in ["---\nflag: [x]\n---","---\nextra: [{deep: {x: a, x: b}}]\n---","---\na: b\na: c\n---","---\na: b","---\na: [broken\n---"] {
            db.connection.execute("UPDATE notes SET body=?1 WHERE id=?2",params![invalid,note])?;
            assert!(db.import_obsidian_base(root,"bad.base",raw.as_bytes().to_vec()).is_err());
        }
        assert_eq!(db.collections()?.len(),1);
        Ok(())
    }
    #[test]
    fn existing_note_field_is_preserved_and_unverified_frontmatter_rolls_back() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let root=db.import_vault("庫",vec![page("甲")])?;
        let note=db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1",[root],|r|r.get::<_,i64>(0))?;
        db.set_property(note,"file.folder",Some("私人值"))?;
        let (_,first)=db.import_obsidian_base(root,"safe.base",b"views: [{type: table, name: Safe, order: [file.folder]}]".to_vec())?;
        assert_eq!(db.connection.query_row("SELECT value FROM obsidian_base_file_values WHERE collection_id=?1 AND note_id=?2 AND key='file.folder'",params![first[0].0,note],|r|r.get::<_,String>(0))?,"甲");
        assert_eq!(db.connection.query_row("SELECT value FROM note_properties WHERE note_id=?1 AND key='file.folder'",[note],|r|r.get::<_,String>(0))?,"私人值");
        db.connection.execute("INSERT INTO heptabase_import_roots(import_root) VALUES(?1)",[root])?;
        db.connection.execute("UPDATE notes SET body='---
title: 誤認頁首
---' WHERE id=?1",[note])?;
        let before=db.collections()?;
        let raw=b"views: [{type: table, name: A, order: [file.name]}, {type: list, name: B, order: [note.title]}]";
        assert!(db.import_obsidian_base(root,"x.base",raw.to_vec()).is_err());
        assert_eq!(db.collections()?,before);
        assert!(!db.connection.prepare("SELECT 1 FROM note_properties WHERE key='file.name'")?.exists([])?);
        assert!(!db.connection.prepare("SELECT 1 FROM attachments WHERE name='x.base'")?.exists([])?);
        Ok(())
    }
}
