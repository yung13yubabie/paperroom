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
        crate::output::windows_output_name(filename.as_ref())?;
        let stem = filename.rsplit_once('.').filter(|(s,e)| !s.trim().is_empty() && e.eq_ignore_ascii_case("base"))
            .map(|(s,_)|s).ok_or("請選擇 .base 原檔。")?;
        if filename.encode_utf16().count()>255 || filename.chars().any(char::is_control)
            || !attachments::has_expected_signature("text",&raw) { return Err("Bases 檔名或文字格式無效。".into()); }
        let base = obsidian_base::parse(&raw)?;
        if live && !base.has_live_status() {return Err("這份 Bases 沒有連結的 status 或 status 等值公式。".into());}
        let tx=Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        let token:String=tx.query_row("SELECT i.token FROM notes n JOIN note_link_identities i ON i.note_id=n.id
            WHERE n.id=?1 AND n.trash_root IS NULL AND EXISTS(SELECT 1 FROM vault_paths WHERE import_root=n.id)",
            [root],|r|r.get(0)).optional()?.ok_or("請選擇仍在使用中的已匯入 Vault 根筆記。")?;
        let used:i64=tx.query_row("SELECT coalesce(sum(length(content)),0) FROM attachments",[],|r|r.get(0))?;
        if used>attachments::MAX_STORAGE_BYTES - base.raw.len() as i64 { return Err("工作區附件空間不足。".into()); }
        let mut report=format!("原始檔：{}\n\n來源 Vault：[[{NATIVE_NOTE_PREFIX}{root}:{token}|開啟來源]]\n\n這些集合使用匯入時的成員與檔案欄位。頁首欄位取自匯入此檢視當下，工作區內已儲存的 Markdown。可由集合選單開啟同名檢視。\n",text_for_receipt(filename));
        let mut created=Vec::new();
        let normalized_handoff:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM heptabase_import_roots WHERE import_root=?1)",[root],|r|r.get(0))?;
        if live && normalized_handoff {return Err("此來源可能曾移除 Markdown 首行，請使用快照模式。".into());}
        if live {report.push_str("\n已連結的 status 欄位使用工作區目前保存的 Markdown；編輯時只更新該筆記。\n");}
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
                let predicate=match col {
                    Column::Front(key) if key=="status"=>Some(None),
                    Column::Formula(expr)=>expr.status_literal().map(Some),
                    _=>None
                };
                if live && let Some(equals)=predicate {
                    tx.execute("INSERT INTO obsidian_base_live_columns(collection_id,key,import_root,equals_text) VALUES(?1,?2,?3,?4)",params![collection,key,root,equals])?;
                }
            }
            let mut members=Vec::new();
            {
                let mut stmt=tx.prepare_cached("SELECT p.note_id,p.relative_path FROM vault_paths p JOIN notes n ON n.id=p.note_id
                    WHERE p.import_root=?1 AND n.trash_root IS NULL ORDER BY p.relative_path")?;
                let mut rows=stmt.query([root])?;
                while let Some(row)=rows.next()? {
                    let path=row.get_ref(1)?.as_str()?;
                    if !view.matches_path(path) { continue; }
                    if members.len()>=MAX_ITEMS as usize { return Err("檢視超過 1,000 篇筆記。".into()); }
                    members.push((row.get::<_,i64>(0)?,path.to_owned()));
                }
            }
            let selected: std::collections::BTreeSet<_> = cols.iter().filter_map(|(_,col)| match col {
                Column::Front(key) if !live || key != "status" => Some(key.as_str()),
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
                for (label,col) in &cols {
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
            let labels:Vec<_>=cols.iter().map(|(key,_)|key).collect();
            tx.execute("INSERT INTO collection_display_columns(collection_id,columns_json) VALUES(?1,?2)",
                params![collection,serde_json::to_string(&labels)?])?;
            report.push_str(&format!("\n- 集合 #{}　{}（{} 篇）",collection,text_for_receipt(&name),members.len()));
            created.push((collection,name,members.len() as i64,list));
        }
        let receipt=insert(&tx,Some(root),&format!("Bases 來源：{filename}"),&report)?;
        tx.execute("INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,?2,'text',?3)",
            params![receipt,filename,base.raw])?;
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
