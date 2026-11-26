//! Explicitly selected rows of an ordinary collection share a single write transaction.
use super::{CollectionView,FieldEdit,Notebook,PropertyType,Result};
use rusqlite::{Transaction,TransactionBehavior,params};
use std::collections::{HashMap,HashSet};

#[derive(Debug,PartialEq,Eq)]
pub(crate) struct BulkRow {
    pub id:i64, pub title:String, pub previous:Option<String>,
    status:String, token:String, version:i64,
}
pub(crate) struct BulkEdit {
    collection:i64, view:Option<CollectionView>, pub key:String, pub kind:PropertyType,
    pub rows:Vec<BulkRow>,
}

impl Notebook {
    pub(crate) fn bulk_field_choices(&self,collection:i64)->Result<Vec<(String,PropertyType)>> {
        let mut q=self.connection.prepare_cached(
            "SELECT d.key,d.kind FROM property_definitions d WHERE NOT EXISTS
             (SELECT 1 FROM obsidian_base_file_columns f WHERE f.collection_id=? AND f.key=d.key) ORDER BY d.key")?;
        Ok(q.query_map([collection],|r|Ok((r.get(0)?,PropertyType::decode(r.get_ref(1)?.as_str()?)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn batch_rows(&self,collection:i64,view:Option<&CollectionView>,key:&str,kind:PropertyType)->Result<Vec<BulkRow>> {
        super::super::obsidian_base_dynamic::require_manual_membership(&self.connection,collection)?;
        if let Some(view)=view {
            if view.presentation.calendar_key.is_some() || view.presentation.timeline_mode {
                return Err("請開啟一般集合表格、清單或看板再批次編輯。".into());
            }
            if !self.collection_views(collection)?.contains(view) {
                return Err("集合檢視已變更，請重新整理。".into());
            }
        }
        if !self.bulk_field_choices(collection)?.iter().any(|(k,t)| k==key && *t==kind) {return Err("屬性型別已變更。".into());}
        let folds=view.map(|v|self.collection_group_folds(v)).transpose()?.flatten();
        let visible=self.presented_collection_lanes(collection,view)?.into_iter()
            .filter(|(group,_)| !folds.as_ref().is_some_and(|f|f.contains(group)))
            .flat_map(|(_,rows)|rows.into_iter().map(|r|r.id)).collect::<Vec<_>>();
        let mut query=self.connection.prepare_cached(
            "SELECT n.id,n.title,i.token,n.version,c.status,p.value FROM collection_notes c
             JOIN notes n ON n.id=c.note_id JOIN note_link_identities i ON i.note_id=n.id
             LEFT JOIN note_properties p ON p.note_id=n.id AND p.key=?2
             WHERE c.collection_id=?1 AND n.trash_root IS NULL")?;
        let mut members=query.query_map(params![collection,key],|r|{
            let row=BulkRow{id:r.get(0)?,title:r.get(1)?,token:r.get(2)?,version:r.get(3)?,status:r.get(4)?,previous:r.get(5)?};
            Ok((row.id,row))
        })?.collect::<rusqlite::Result<HashMap<_,_>>>()?;
        visible.into_iter().map(|id|members.remove(&id).ok_or_else(||"資料列已變更。".into())).collect()
    }

    pub(crate) fn begin_bulk_field_edit(&self,collection:i64,view:Option<&CollectionView>,key:&str,kind:PropertyType)->Result<BulkEdit> {
        let tx=self.connection.unchecked_transaction()?;
        let rows=self.batch_rows(collection,view,key,kind)?;
        tx.commit()?;
        Ok(BulkEdit {collection,view:view.cloned(),key:key.into(),kind,rows})
    }

    pub(crate) fn commit_bulk_field_edit(&self,edit:&BulkEdit,ids:&[i64],value:Option<&str>)->Result<usize> {
        if !(2..=100).contains(&ids.len()) || ids.iter().collect::<HashSet<_>>().len()!=ids.len() {
            return Err("請明確選擇 2 至 100 筆不同的資料列。".into());
        }
        if let Some(v)=value && (v.len()>super::super::MAX_VALUE_BYTES || edit.kind.normalized(v).is_none()){
            return Err(format!("請輸入{}。",edit.kind.hint()).into());
        }
        let tx=Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        // Check the entire selection against the pre-write view. A successful edit may
        // legitimately make every selected row disappear from a filter or change groups.
        let current=self.batch_rows(edit.collection,edit.view.as_ref(),&edit.key,edit.kind)?
            .into_iter().map(|r|(r.id,r)).collect::<HashMap<_,_>>();
        let expected=edit.rows.iter().map(|r|(r.id,r)).collect::<HashMap<_,_>>();
        let selected=ids.iter().map(|id|{
            match (current.get(id),expected.get(id)) {
                (Some(now),Some(&old)) if now==old=>Ok(old),
                _=>Err(format!("資料列 #{id} 已變更或已隱藏，請重新選取。")),
            }
        }).collect::<std::result::Result<Vec<_>,_>>()?;
        let mut changed=0;
        for row in selected {
            if row.previous.as_deref()==value {continue;}
            Self::write_member_field(&tx,edit.collection,row.id,&row.status,FieldEdit{
                key:&edit.key,kind:edit.kind,previous:row.previous.as_deref(),value,
            })?;
            changed+=1;
        }
        tx.commit()?;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{CollectionLayout,PropertyFilter};
    use super::super::super::collection_field_rules::CollectionFieldRule;
    use std::fs;

    #[test]
    fn batch_refuses_newly_folded_rows_and_fields_shadowed_by_imports()->Result<()> {
        use super::super::super::collection_group_folds::GroupValue;
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let c=db.create_collection("普通集合")?;
        let kind=PropertyType::Date;
        let key="Day";
        db.compare_set_property_type(key,None,Some(kind))?;
        let mut ids=Vec::new();
        for status in ["Open","Closed"] {
            let id=db.add(None,"same",status)?;
            db.add_to_collection(c,id,status)?;
            ids.push(id);
        }
        let view=db.create_collection_view(c,"Board",CollectionLayout::Board,"","")?;
        let snap=db.begin_bulk_field_edit(c,Some(&view),key,kind)?;
        assert!(db.commit_bulk_field_edit(&snap,&ids,Some("2027-02-29")).is_err());
        let folds=db.collection_group_folds(&view)?.unwrap();
        db.set_collection_group_fold(c,&view,&folds,Some((&GroupValue(Some("Closed".into())),true)))?;
        assert!(db.commit_bulk_field_edit(&snap,&ids,Some("2028-02-29")).is_err());
        assert!(db.properties(ids[0])?.is_empty());
        let folds=db.collection_group_folds(&view)?.unwrap();
        db.set_collection_group_fold(c,&view,&folds,None)?;
        assert_eq!(db.commit_bulk_field_edit(&snap,&ids,Some("2028-02-29"))?,2);
        let snap=db.begin_bulk_field_edit(c,Some(&view),key,kind)?;
        db.connection.execute("INSERT INTO obsidian_base_file_columns(collection_id,key) VALUES(?,?)",params![c,key])?;
        assert!(db.bulk_field_choices(c)?.is_empty());
        assert!(db.commit_bulk_field_edit(&snap,&ids,None).is_err());
        assert_eq!(db.properties(ids[1])?[0].1,"2028-02-29");
        Ok(())
    }

    #[test]
    fn bulk_selected_fields_are_atomic_and_keep_note_history_links_assets_across_restore()->Result<()> {
        let dir=super::super::super::tests::data_dir();
        let path=dir.join("work.sqlite3");
        let mut db=Notebook::open(&path,false)?;
        let c=db.create_collection("Scope")?;
        let extra=db.create_collection("Other")?;
        let key="工作量";
        db.compare_set_property_type(key,None,Some(PropertyType::Number))?;
        let mut ids=Vec::new();
        for (body,status) in [("A","ready"),("B","ready"),("C","ready"),("D","hold")] {
            let note=db.add(None,"同名",body)?;
            db.add_to_collection(c,note,status)?;
            db.compare_set_property(note,key,None,Some("2"))?;
            ids.push(note);
        }
        let (a,b,spare,hidden)=(ids[0],ids[1],ids[2],ids[3]);
        db.add_to_collection(extra,b,"other")?;
        db.set_relation(a,"引文",b,true)?;
        let input=dir.join("演出.txt");fs::write(&input,b"exact attachment bytes")?;
        let file=db.import_attachment(a,&input)?;
        let view=db.save_collection_view(c,"Ready",CollectionLayout::Table,"ready","",
            Some(PropertyFilter{key:key.into(),kind:PropertyType::Number,value:"2".into()}))?;
        let snap=db.begin_bulk_field_edit(c,Some(&view),key,PropertyType::Number)?;
        assert_eq!(snap.rows.len(),3);
        assert!(db.commit_bulk_field_edit(&snap,&[a,hidden],Some("9")).is_err());
        assert!(db.commit_bulk_field_edit(&snap,&[a,a],Some("9")).is_err());
        db.compare_set_property(b,key,Some("2"),Some("4"))?;
        assert!(db.commit_bulk_field_edit(&snap,&[a,b],Some("9")).is_err());
        assert_eq!(db.properties(a)?[0].1,"2");
        db.compare_set_property(b,key,Some("4"),Some("2"))?;
        let snap=db.begin_bulk_field_edit(c,Some(&view),key,PropertyType::Number)?;
        let prev=db.note_revisions(a)?.len();
        assert_eq!(db.commit_bulk_field_edit(&snap,&[a,b],Some("2"))?,0);
        assert_eq!(db.note_revisions(a)?.len(),prev);
        assert_eq!(db.commit_bulk_field_edit(&snap,&[a,b],Some("0"))?,2);
        assert_eq!(db.filtered_collection_rows(c,Some(&view))?[0].0.id,spare);
        assert_eq!(db.note_revisions(a)?.len(),prev+1);
        db.save_collection_field_rule(extra,&CollectionFieldRule{key:key.into(),kind:PropertyType::Number,required:true,default:None},None)?;
        let clear=db.begin_bulk_field_edit(c,None,key,PropertyType::Number)?;
        let before=db.note_revisions(a)?.len();
        assert!(db.commit_bulk_field_edit(&clear,&[a,b],None).is_err());
        assert_eq!(db.note_revisions(a)?.len(),before);
        assert_eq!(db.properties(a)?[0].1,"0");
        assert_eq!(db.commit_bulk_field_edit(&clear,&[a,spare],None)?,2);
        drop(db);
        let db=Notebook::open(&path,false)?;
        let backup=dir.join("backup.sqlite3");db.backup(&backup)?;drop(db);
        let isolated=dir.join("restore.sqlite3");fs::copy(backup,&isolated)?;
        let db=Notebook::open(&isolated,false)?;
        assert!(db.properties(a)?.is_empty() && db.properties(spare)?.is_empty());
        assert_eq!(db.properties(b)?[0].1,"0");
        assert_eq!(db.properties(hidden)?[0].1,"2");
        assert_eq!(db.note(a)?.unwrap().body,"A");
        assert_eq!(db.relations(a)?[0].1.id,b);
        assert!(db.collection_views(c)?.contains(&view));
        let copy=dir.join("out.txt");db.save_attachment_copy(file,&copy)?;
        assert_eq!(fs::read(copy)?,b"exact attachment bytes");
        drop(db);fs::remove_dir_all(dir)?;Ok(())
    }
}
