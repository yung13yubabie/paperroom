//! Explicit path-to-receipt bindings for named Bases cards in a Vault Canvas.
use super::{Notebook,Result,collection_views::CollectionView};
use crate::canvas::Document;
use rusqlite::{Transaction,TransactionBehavior,OptionalExtension,params};

pub(super) const SCHEMA:&str="
CREATE TABLE IF NOT EXISTS base_card_receipts(
 receipt INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
 import_root INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
 attachment INTEGER NOT NULL UNIQUE REFERENCES attachments(id) ON DELETE CASCADE,
 filename TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS base_card_views(
 receipt INTEGER NOT NULL REFERENCES base_card_receipts(receipt) ON DELETE CASCADE,
 name TEXT NOT NULL, view_id INTEGER NOT NULL UNIQUE REFERENCES collection_views(id) ON DELETE CASCADE,
 PRIMARY KEY(receipt,name)
);
CREATE TABLE IF NOT EXISTS canvas_base_paths(
 canvas_id INTEGER NOT NULL REFERENCES saved_canvases(id) ON DELETE CASCADE,
 import_root INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
 file TEXT NOT NULL, receipt INTEGER NOT NULL REFERENCES base_card_receipts(receipt) ON DELETE CASCADE,
 PRIMARY KEY(canvas_id,import_root,file), UNIQUE(canvas_id,import_root,receipt)
);";

pub(crate) fn named_nodes(doc:&Document)->Vec<(&str,&str,&str)> {
    doc.json()["nodes"].as_array().into_iter().flatten().filter_map(|n|{
        if n["type"]!="file" {return None;}
        let path=n["file"].as_str()?;
        let name=n["subpath"].as_str()?.strip_prefix('#')?;
        (super::notion_handoff::windows_zip_path(path) && path.rsplit_once('.').is_some_and(|(_,e)|e.eq_ignore_ascii_case("base"))
            && !name.is_empty() && !name.chars().any(char::is_control)).then_some((n["id"].as_str()?,path,name))
    }).collect()
}

const ACTIVE:&str=" FROM base_card_receipts s JOIN attachments a ON a.id=s.attachment AND a.note_id=s.receipt AND a.name=s.filename COLLATE BINARY
 JOIN notes n ON n.id=s.receipt AND n.trash_root IS NULL JOIN notes r ON r.id=s.import_root AND r.trash_root IS NULL ";

impl Notebook {
    fn check_base_card(&self,id:i64,doc:&Document,root:i64,node:&str)->Result<(String,String)> {
        if self.canvas_vault(id)?!=Some(root) || self.read_canvas(id)?.as_str()!=doc.as_str() {
            return Err("來源 Vault 或畫布已變更，請重新開啟畫布。".into());
        }
        named_nodes(doc).into_iter().find(|(key,_,_)| *key==node)
            .map(|(_,file,name)|(file.into(),name.into())).ok_or_else(||"此節點沒有完整的 Bases 檔案及具名檢視。".into())
    }

    fn matching_sources(&self,root:i64,file:&str,bytes:Option<&[u8]>)->Result<Vec<i64>> {
        let sql=format!("SELECT s.receipt {ACTIVE} WHERE s.import_root=?1 AND s.filename=?2 COLLATE BINARY
            AND (?3 IS NULL OR a.content=?3) ORDER BY s.receipt");
        let rows=self.connection.prepare(&sql)?.query_map(params![root,file.rsplit('/').next(),bytes],|r|r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub(crate) fn base_card_needs_file(&self,root:i64,file:&str)->Result<bool> {
        Ok(file.contains('/') || self.matching_sources(root,file,None)?.len()>1)
    }

    fn verify_source(&self,root:i64,file:&str,proof:Option<&[u8]>)->Result<i64> {
        if proof.is_none() && self.base_card_needs_file(root,file)? {
            return Err("請選取原始 Vault，以完整路徑和原檔內容核對來源。".into());
        }
        match self.matching_sources(root,file,proof)?.as_slice() {
            [receipt]=>Ok(*receipt),
            []=>Err("找不到相符的來源收據；請核對 Vault 或重新匯入原始檔。".into()),
            _=>Err("多份收據的來源仍無法區分，未配對。".into()),
        }
    }

    pub(crate) fn base_card_candidate(&self,id:i64,doc:&Document,root:i64,node:&str,proof:Option<&[u8]>)->Result<i64> {
        let tx=self.connection.unchecked_transaction()?;
        let (file,name)=self.check_base_card(id,doc,root,node)?;
        let receipt=self.verify_source(root,&file,proof)?;
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM base_card_views WHERE receipt=?1 AND name=?2)",
            params![receipt,name], |r|r.get::<_,bool>(0))? { return Err("這份來源沒有該具名檢視。".into()); }
        tx.commit()?;
        Ok(receipt)
    }

    pub(crate) fn bind_base_card(&self,id:i64,doc:&Document,root:i64,node:&str,receipt:i64,proof:Option<&[u8]>)->Result<()> {
        let tx=Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        let (file,view)=self.check_base_card(id,doc,root,node)?;
        if self.verify_source(root,&file,proof)? != receipt { return Err("來源收據已變更，請重新核對。".into()); }
        let changed=tx.execute("INSERT INTO canvas_base_paths(canvas_id,import_root,file,receipt)
            SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM base_card_views WHERE receipt=?4 AND name=?5)
            ON CONFLICT DO NOTHING",params![id,root,file,receipt,view])?;
        if changed!=1{return Err("此來源或路徑已配對，或具名檢視已移除。請核對原配對。".into());}
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn base_path_receipt(&self,id:i64,root:i64,path:&str)->Result<Option<i64>> {
        Ok(self.connection.query_row("SELECT receipt FROM canvas_base_paths WHERE canvas_id=?1 AND import_root=?2 AND file=?3",
            params![id,root,path],|r|r.get(0)).optional()?)
    }

    pub(crate) fn unbind_base_card(&self,id:i64,doc:&Document,root:i64,node:&str,expected:i64)->Result<()> {
        let tx=Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        let (file,_)=self.check_base_card(id,doc,root,node)?;
        if tx.execute("DELETE FROM canvas_base_paths WHERE canvas_id=?1 AND import_root=?2 AND file=?3 AND receipt=?4",
            params![id,root,file,expected])?!=1 {return Err("配對已變更，請重新核對。".into());}
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn resolve_base_card(&self,id:i64,doc:&Document,root:i64,node:&str)->Result<(i64,i64,String,CollectionView)> {
        let tx=self.connection.unchecked_transaction()?;
        let (file,name)=self.check_base_card(id,doc,root,node)?;
        let sql=format!("SELECT s.receipt,c.id,c.name,v.view_id {ACTIVE}
            JOIN canvas_base_paths p ON p.receipt=s.receipt AND p.import_root=s.import_root
            JOIN base_card_views v ON v.receipt=s.receipt JOIN collection_views cv ON cv.id=v.view_id
            JOIN collections c ON c.id=cv.collection_id
            WHERE p.canvas_id=?1 AND p.import_root=?2 AND p.file=?3 COLLATE BINARY AND v.name=?4 COLLATE BINARY");
        let (receipt,col,title,view):(i64,i64,String,i64)=tx.query_row(&sql,params![id,root,file,name],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.ok_or("沒有這個原始具名檢視的有效配對。")?;
        let view=self.collection_views(col)?.into_iter().find(|v|v.id==view).ok_or("找不到已保存的具名檢視。")?;
        tx.commit()?;
        Ok((receipt,col,title,view))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use crate::store::canvas_edit::CanvasEdit;
    use serde_json::json;

    #[test]
    fn named_cards_bind_explicit_paths_and_stable_views_in_one_vault() -> Result<()> {
        let dir=std::env::temp_dir().join(format!("paperroom-named-base-cards-{}",std::process::id()));
        std::fs::create_dir(&dir)?;
        let (live,backup)=(dir.join("live.db"),dir.join("backup.db"));
        let mut db=Notebook::open(&live,false)?;
        let pages=||["A","B"].into_iter().map(|s|ImportedPage{parts:vec![s.into(),"鏡頭.md".into()],title:"鏡頭".into(),body:s.into()}).collect();
        let root=db.import_vault("原批",pages())?;
        let other=db.import_vault("另一批",pages())?;
        let yaml=b"views:\n  - type: table\n    name: Pin_A_601\n    filters: 'file.folder == \"A\"'\n    order: [file.path]\n  - type: list\n    name: Pin_B_904\n    filters: 'file.folder == \"B\"'\n    order: [file.path]\n";
        let (receipt,cols)=db.import_obsidian_base(root,"Shipping.base",yaml.into())?;
        for item in &cols {db.connection.execute("UPDATE collections SET name=name||' (1)' WHERE id=?1",[item.0])?;}
        let other_yaml=[yaml.as_slice(),b"\n# other source"].concat();
        let folder=dir.join("測試 Vault"); std::fs::create_dir_all(folder.join("資料"))?;
        std::fs::write(folder.join("資料/Shipping.base"),&other_yaml)?;
        let exact=super::super::canvas_bundle::read_base_card_file(&folder,"資料/Shipping.base")?;
        assert_eq!(exact,other_yaml);
        assert!(super::super::canvas_bundle::read_base_card_file(&folder,"資料/shipping.base").is_err());
        assert!(super::super::canvas_bundle::read_base_card_file(&folder,"資料/../Shipping.base").is_err());
        let (duplicate,second)=db.import_obsidian_base(root,"Shipping.base",other_yaml.clone())?;
        for item in &second {db.connection.execute("UPDATE collections SET name=name||' (2)' WHERE id=?1",[item.0])?;}
        let (unrelated,_)=db.import_obsidian_base(other,"Shipping.base",yaml.into())?;
        let cases=[("a","Shipping.base","#Pin_A_601"),("b","Shipping.base","#Pin_B_904"),("nested","資料/Shipping.base","#Pin_A_601"),
            ("dot","./Shipping.base","#Pin_A_601"),("case","shipping.base","#Pin_A_601"),("wrong","Shipping.base","#pin_a_601")];
        let nodes:Vec<_>=cases.into_iter().map(|(id,file,subpath)|json!({"id":id,"type":"file","file":file,"subpath":subpath,"x":0,"y":0,"width":232,"height":114})).collect();
        let raw=format!("{}\r\n",serde_json::to_string_pretty(&json!({"nodes":nodes}))?);
        let doc=crate::canvas::parse(raw.clone())?;
        let id=db.save_canvas("白板",&doc)?;
        db.set_canvas_vault(id,Some(root))?;
        assert!(db.base_card_candidate(id,&doc,root,"a",None).is_err());
        assert_eq!(db.base_card_candidate(id,&doc,root,"a",Some(yaml))?,receipt);
        assert!(db.base_card_candidate(id,&doc,root,"nested",Some(b"changed")).is_err());
        db.bind_base_card(id,&doc,root,"nested",duplicate,Some(&exact))?;
        assert_eq!(db.resolve_base_card(id,&doc,root,"nested")?.0,duplicate);
        assert!(db.unbind_base_card(id,&doc,root,"nested",receipt).is_err());
        db.unbind_base_card(id,&doc,root,"nested",duplicate)?;
        assert!(db.resolve_base_card(id,&doc,root,"a").is_err());
        assert!(db.bind_base_card(id,&doc,root,"dot",receipt,Some(yaml)).is_err());
        assert!(db.bind_base_card(id,&doc,root,"case",receipt,Some(yaml)).is_err());
        assert!(db.bind_base_card(id,&doc,root,"a",unrelated,Some(yaml)).is_err());
        db.bind_base_card(id,&doc,root,"a",receipt,Some(yaml))?;
        assert!(db.bind_base_card(id,&doc,root,"nested",receipt,Some(yaml)).is_err());
        assert!(db.bind_base_card(id,&doc,root,"b",duplicate,Some(yaml)).is_err());
        for node in ["nested","wrong"] {assert!(db.resolve_base_card(id,&doc,root,node).is_err());}
        let changed=db.edit_canvas(id,&doc,CanvasEdit::Content{id:"a",value:"別圖/Shipping.base"})?;
        assert!(db.resolve_base_card(id,&doc,root,"a").is_err());
        assert!(db.resolve_base_card(id,&changed,root,"a").is_err());
        db.undo_canvas_edit(id,&changed)?.unwrap();
        db.set_canvas_vault(id,Some(other))?;
        assert!(db.resolve_base_card(id,&doc,other,"a").is_err());
        assert!(db.resolve_base_card(id,&doc,root,"a").is_err());
        db.set_canvas_vault(id,Some(root))?;
        db.trash(receipt)?;
        assert!(db.resolve_base_card(id,&doc,root,"a").is_err());
        db.restore(receipt)?;
        db.backup(&backup)?;
        drop(db);
        for dbfile in [&live,&backup] {
            let db=Notebook::open(dbfile,false)?;
            assert_eq!(db.read_canvas(id)?.as_str(),raw);
            for (node,idx,body) in [("a",0,"A"),("b",1,"B")] {
                let (chosen,col,_,view)=db.resolve_base_card(id,&doc,root,node)?;
                assert_eq!(chosen,receipt);
                assert_eq!(col,cols[idx].0);
                let rows=db.filtered_collection_rows(col,Some(&view))?;
                assert_eq!(rows.len(),1);
                assert_eq!(db.note(rows[0].0.id)?.unwrap().body,body);
            }
            let (_,col,_,v)=db.resolve_base_card(id,&doc,root,"b")?;
            db.delete_collection_view(col,v.id)?;
            assert!(db.resolve_base_card(id,&doc,root,"b").is_err());
            assert!(db.resolve_base_card(id,&doc,root,"a").is_ok());
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
