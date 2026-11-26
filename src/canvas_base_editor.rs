//! All fields in an editing prompt belong to the selected Canvas occurrence.
use super::*;
use crate::store::obsidian_base_typed::{edit_typed_base_on,existing_direct_value};

#[derive(PartialEq,Eq)]
pub(crate) struct Pin {
    canvas:i64, root:i64, node:String, receipt:i64, receipt_token:String, collection:i64, view:CollectionView,
}
#[derive(PartialEq,Eq)]
pub(crate) struct PinRow {
    pub(crate) id:i64, pub(crate) title:String, pub(crate) path:String, token:String, version:i64,
}
#[derive(PartialEq,Eq)]
pub(crate) struct PinField {
    pub(crate) key:String, pub(crate) kind:String, pub(crate) value:String, source:String,
}

impl Notebook {
    fn base_edit_pin(&self,canvas:i64,doc:&Document,root:i64,node:&str)->Result<Pin>{
        let (receipt,collection,_,view)=self.resolve_base_card_in(canvas,doc,root,node)?;
        let receipt_token=self.connection.query_row("SELECT token FROM note_link_identities WHERE note_id=?1",[receipt],|r|r.get(0))?;
        Ok(Pin{canvas,root,node:node.into(),receipt,receipt_token,collection,view})
    }

    fn pin_rows(&self,pin:&Pin)->Result<Vec<PinRow>>{
        let mut query=self.connection.prepare_cached("SELECT i.token,n.version,p.relative_path FROM note_link_identities i
            JOIN notes n ON n.id=i.note_id JOIN vault_paths p ON p.note_id=n.id
            WHERE n.id=?1 AND p.import_root=?2 AND n.trash_root IS NULL")?;
        let mut rows=Vec::new();
        for (row,_) in self.filtered_collection_rows(pin.collection,Some(&pin.view))? {
            if let Some((token,version,path))=query.query_row(params![row.id,pin.root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()? {
                rows.push(PinRow{id:row.id,title:row.title,path,token,version});
            }
        }
        Ok(rows)
    }

    pub(crate) fn start_pin_edit(&self,canvas:i64,doc:&Document,root:i64,node:&str)->Result<(Pin,Vec<PinRow>)>{
        let tx=self.connection.unchecked_transaction()?;
        let pin=self.base_edit_pin(canvas,doc,root,node)?;
        let rows=self.pin_rows(&pin)?;
        tx.commit()?;
        Ok((pin,rows))
    }

    fn pin_fields_in(&self,doc:&Document,pin:&Pin,row:&PinRow)->Result<Vec<PinField>>{
        if self.base_edit_pin(pin.canvas,doc,pin.root,&pin.node)?!=*pin || !self.pin_rows(pin)?.contains(row){
            return Err("卡片來源、檢視或資料列已變更，請重新選取。".into());
        }
        let columns=self.collection_display_columns(pin.collection)?.unwrap_or_default();
        let body:String=self.connection.query_row("SELECT body FROM notes WHERE id=?1",[row.id],|r|r.get(0))?;
        let mut query=self.connection.prepare_cached("SELECT b.source_key,b.kind FROM obsidian_base_typed_columns b
            JOIN obsidian_base_file_values f ON f.collection_id=b.collection_id AND f.key=b.key
            WHERE b.import_root=?1 AND b.collection_id=?2 AND b.key=?3 AND b.op IS NULL AND f.note_id=?4")?;
        let mut fields=Vec::new();
        for key in columns {
            let binding:Option<(String,String)>=query.query_row(params![pin.root,pin.collection,key,row.id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((source,kind))=binding && let Some(value)=existing_direct_value(&body,&source,&kind) {
                fields.push(PinField{key,kind,value,source});
            }
        }
        Ok(fields)
    }

    pub(crate) fn pin_fields(&self,doc:&Document,pin:&Pin,row:&PinRow)->Result<Vec<PinField>>{
        let tx=self.connection.unchecked_transaction()?;
        let fields=self.pin_fields_in(doc,pin,row)?;
        tx.commit()?;
        Ok(fields)
    }

    pub(crate) fn save_pin_field(&self,doc:&Document,pin:&Pin,row:&PinRow,field:&PinField,draft:&str)->Result<()>{
        let tx=Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        if !self.pin_fields_in(doc,pin,row)?.contains(field){
            return Err("此欄位的來源、值或顯示設定已變更，未儲存。".into());
        }
        edit_typed_base_on(&tx,pin.collection,&field.key,row.id,row.version,draft)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use crate::store::canvas_edit::CanvasEdit;

    #[test]
    fn card_edits_write_only_current_visible_source_and_leave_canvas_bytes_intact()->Result<()> {
        let dir=std::env::temp_dir().join(format!("paperroom-base-card-edit-{}",std::process::id()));
        std::fs::create_dir(&dir)?;
        let (file,backup)=(dir.join("main.db"),dir.join("backup.db"));
        let mut db=Notebook::open(&file,false)?;
        let body="---\napproved: true  # 保留\ncost: 2.50\n---\n原始正文";
        let root=db.import_vault("分鏡",["甲","乙"].into_iter().map(|folder|ImportedPage{
            title:"同名鏡頭".into(),parts:vec![folder.into(),"分鏡.md".into()],body:body.into()}).collect())?;
        let yaml="filters: 'note.approved == true'\nformulas: {more: 'note.cost >= 0'}\nviews:\n  - type: table\n    name: 分鏡\n    order: [file.path,approved,cost,formula.more]\n";
        let (receipt,_)=db.import_obsidian_base_options(root,"場記.base",yaml.as_bytes().to_vec(),true,true)?;
        let raw=r##"{"nodes":[{"id":"pin1","type":"file","file":"場記.base","subpath":"#分鏡","x":12,"y":14,"width":342,"height":215,"custom":{"保留":1}}]}"##;
        let doc=crate::canvas::parse(raw.into())?;
        let id=db.save_canvas("白板",&doc)?;
        db.set_canvas_vault(id,Some(root))?;
        db.bind_base_card(id,&doc,root,"pin1",receipt,None)?;
        let writer=Notebook::open(&file,false)?;
        let (pin,rows)=db.start_pin_edit(id,&doc,root,"pin1")?;
        let row=rows.iter().find(|r|r.path.starts_with("甲/")).unwrap();
        let decoy=rows.iter().find(|r|r.path.starts_with("乙/")).unwrap().id;
        let note=row.id;
        let fields=db.pin_fields(&doc,&pin,row)?;
        assert_eq!(fields.len(),2);
        let cost=fields.iter().find(|f|f.key=="cost").unwrap();
        let orig=db.note(note)?.unwrap();
        assert!(db.save_pin_field(&doc,&pin,row,cost,"bad").is_err());
        assert_eq!(db.note(note)?.unwrap().version,orig.version);
        let columns=writer.collection_display_columns(pin.collection)?.unwrap();
        let hidden=columns.iter().filter(|key| key.as_str()!="cost").cloned().collect::<Vec<_>>();
        writer.set_collection_display_columns(pin.collection,Some(&columns),Some(&hidden))?;
        assert!(db.save_pin_field(&doc,&pin,row,cost,"3").is_err());
        assert!(!db.pin_fields(&doc,&pin,row)?.iter().any(|f|f.key=="cost"));
        writer.set_collection_display_columns(pin.collection,Some(&hidden),Some(&columns))?;
        assert_eq!(db.note(note)?.unwrap().version,orig.version);
        writer.unbind_base_card(id,&doc,root,"pin1",receipt)?;
        assert!(db.save_pin_field(&doc,&pin,row,cost,"3").is_err());
        writer.bind_base_card(id,&doc,root,"pin1",receipt,None)?;
        let moved=writer.edit_canvas(id,&doc,CanvasEdit::Geometry{id:"pin1",geometry:[10,15,656,300]})?;
        assert!(db.save_pin_field(&doc,&pin,row,cost,"3").is_err());
        writer.undo_canvas_edit(id,&moved)?;
        writer.save(note,orig.version,"同名鏡頭",&format!("{body}\n另筆修改"))?;
        assert!(db.save_pin_field(&doc,&pin,row,cost,"3").is_err());
        let (pin,rows)=db.start_pin_edit(id,&doc,root,"pin1")?;
        let row=rows.iter().find(|r|r.id==note).unwrap();
        let fields=db.pin_fields(&doc,&pin,row)?;
        let cost=fields.iter().find(|f|f.key=="cost").unwrap();
        db.save_pin_field(&doc,&pin,row,cost,"7.25")?;
        let (pin,rows)=db.start_pin_edit(id,&doc,root,"pin1")?;
        let row=rows.iter().find(|r|r.id==note).unwrap();
        let fields=db.pin_fields(&doc,&pin,row)?;
        assert!(fields.iter().any(|f|f.key=="cost" && f.value=="7.25"));
        let approved=fields.iter().find(|f|f.key=="approved").unwrap();
        db.save_pin_field(&doc,&pin,row,approved,"false")?;
        assert!(db.save_pin_field(&doc,&pin,row,approved,"true").is_err());
        db.backup(&backup)?;
        drop(db);drop(writer);
        for path in [&file,&backup]{
            let db=Notebook::open(path,false)?;
            let (_,rows)=db.start_pin_edit(id,&doc,root,"pin1")?;
            assert_eq!(rows.iter().map(|r|r.id).collect::<Vec<_>>(),[decoy]);
            assert_eq!(db.note(decoy)?.unwrap().body,body);
            assert_eq!(db.note(note)?.unwrap().body,"---\napproved: false  # 保留\ncost: 7.25\n---\n原始正文\n另筆修改");
            assert_eq!(db.read_canvas(id)?.as_str(),raw);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
