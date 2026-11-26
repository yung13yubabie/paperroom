//! Collection-scoped relation fields. Both editors operate on the same directed value.
use super::{Notebook, Note, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS relation_fields(
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 source_collection INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
 target_collection INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
 name TEXT NOT NULL COLLATE NOCASE,
 UNIQUE(source_collection,name), UNIQUE(id,source_collection,target_collection));
CREATE INDEX IF NOT EXISTS relation_fields_destination ON relation_fields(target_collection);
CREATE TABLE IF NOT EXISTS relation_field_values(
 field_id INTEGER NOT NULL,source_collection INTEGER NOT NULL,target_collection INTEGER NOT NULL,
 source_note INTEGER NOT NULL,target_note INTEGER NOT NULL,
 PRIMARY KEY(field_id,source_note,target_note),CHECK(source_note<>target_note),
 FOREIGN KEY(field_id,source_collection,target_collection)
 REFERENCES relation_fields(id,source_collection,target_collection) ON DELETE CASCADE,
 FOREIGN KEY(source_collection,source_note) REFERENCES collection_notes(collection_id,note_id) ON DELETE CASCADE,
 FOREIGN KEY(target_collection,target_note) REFERENCES collection_notes(collection_id,note_id) ON DELETE CASCADE);
CREATE INDEX IF NOT EXISTS relation_field_values_target ON relation_field_values(field_id,target_note);";

#[derive(Clone,PartialEq,Eq)]
pub(crate) struct Member { pub id:i64, pub title:String, token:String }
#[derive(Clone,PartialEq,Eq)]
pub(crate) struct Field {
    pub id:i64, pub name:String, pub reverse:bool, pub other:String,
    source:i64, target:i64,
}

fn member(db:&Connection, col:i64, note:i64)->Result<Member> {
    db.query_row("SELECT n.id,n.title,i.token FROM collection_notes m JOIN notes n ON n.id=m.note_id
        JOIN note_link_identities i ON i.note_id=n.id WHERE m.collection_id=? AND n.id=? AND n.trash_root IS NULL",
        params![col,note], |r|Ok(Member{id:r.get(0)?,title:r.get(1)?,token:r.get(2)?})).optional()?
        .ok_or_else(|| "筆記已離開集合或移到垃圾桶。".into())
}
impl Notebook {
    pub(crate) fn relation_collections(&self,note:i64)->Result<Vec<(i64,String,Member)>> {
        let mut q=self.connection.prepare("SELECT c.id,c.name,n.title,i.token FROM collections c
            JOIN collection_notes m ON m.collection_id=c.id JOIN notes n ON n.id=m.note_id
            JOIN note_link_identities i ON i.note_id=n.id WHERE n.id=? AND n.trash_root IS NULL AND NOT EXISTS
              (SELECT 1 FROM obsidian_base_dynamic d WHERE d.collection_id=c.id) ORDER BY c.name,c.id")?;
        Ok(q.query_map([note],|r|Ok((r.get(0)?,r.get(1)?,Member{id:note,title:r.get(2)?,token:r.get(3)?})))?
           .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn relation_target_collections(&self)->Result<Vec<(i64,String)>> {
        let mut query=self.connection.prepare("SELECT id,name FROM collections c WHERE NOT EXISTS
            (SELECT 1 FROM obsidian_base_dynamic d WHERE d.collection_id=c.id) ORDER BY name,id")?;
        Ok(query.query_map([],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn create_relation_field(&self,origin:&Member,source:(i64,&str),target:(i64,&str),name:&str)->Result<()> {
        let name=name.trim();
        if name.is_empty() || name.len()>256 || name.chars().any(char::is_control) {
            return Err("請輸入 1 至 256 位元組的關聯屬性名稱。".into());
        }
        let tx=rusqlite::Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        if member(&tx,source.0,origin.id)? != *origin {return Err("來源筆記已變更。".into());}
        for (id,title) in [source,target] {
            super::obsidian_base_dynamic::require_manual_membership(&tx,id)?;
            if !tx.query_row("SELECT EXISTS(SELECT 1 FROM collections WHERE id=? AND name=? COLLATE BINARY)",
                params![id,title],|r|r.get::<_,bool>(0))? { return Err("集合已變更。".into()); }
        }
        if tx.query_row("SELECT count(*) FROM relation_fields WHERE source_collection=?",[source.0],|r|r.get::<_,i64>(0))? >=128 {
            return Err("每個集合的關聯屬性最多 128 欄。".into());
        }
        if tx.execute("INSERT INTO relation_fields(source_collection,target_collection,name) VALUES(?,?,?)
            ON CONFLICT(source_collection,name) DO NOTHING",params![source.0,target.0,name])?!=1 {
            return Err("來源集合已有同名關聯屬性。".into());
        }
        tx.commit()?; Ok(())
    }

    pub(crate) fn editable_relation_fields(&self,note:i64)->Result<Vec<(Field,Member)>> {
        let mut q=self.connection.prepare("WITH roles(rev) AS (VALUES(0),(1))
            SELECT f.id,f.name,roles.rev,f.source_collection,f.target_collection,c.name,n.title,i.token
            FROM roles CROSS JOIN relation_fields f
            JOIN collection_notes m ON m.collection_id=CASE roles.rev WHEN 0 THEN f.source_collection ELSE f.target_collection END
            JOIN collections c ON c.id=CASE roles.rev WHEN 1 THEN f.source_collection ELSE f.target_collection END
            JOIN notes n ON n.id=m.note_id JOIN note_link_identities i ON i.note_id=n.id
            WHERE n.id=? AND n.trash_root IS NULL ORDER BY roles.rev,c.name,f.name,f.id")?;
        Ok(q.query_map([note],|r|Ok((Field{id:r.get(0)?,name:r.get(1)?,reverse:r.get(2)?,
            source:r.get(3)?,target:r.get(4)?,other:r.get(5)?},Member{id:note,title:r.get(6)?,token:r.get(7)?})))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn field_candidates(&self,field:&Field,origin:&Member,linked:bool)->Result<Vec<Member>> {
        super::obsidian_base_dynamic::require_manual_membership(&self.connection,field.source)?;
        super::obsidian_base_dynamic::require_manual_membership(&self.connection,field.target)?;
        let (local,other)=if field.reverse {(field.target,field.source)} else {(field.source,field.target)};
        if member(&self.connection,local,origin.id)? != *origin {return Err("目前筆記已變更。".into());}
        let (a,b)=if field.reverse {("target_note","source_note")}else{("source_note","target_note")};
        let sql=format!("SELECT n.id,n.title,i.token FROM collection_notes m JOIN notes n ON n.id=m.note_id
            JOIN note_link_identities i ON i.note_id=n.id WHERE m.collection_id=?1 AND n.id<>?2 AND n.trash_root IS NULL
            AND EXISTS(SELECT 1 FROM relation_field_values v WHERE v.field_id=?3 AND v.{a}=?2 AND v.{b}=n.id)=?4
            ORDER BY n.title,n.id");
        let mut q=self.connection.prepare(&sql)?;
        Ok(q.query_map(params![other,origin.id,field.id,linked],|r|Ok(Member{id:r.get(0)?,title:r.get(1)?,token:r.get(2)?}))?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn check_field_pair(&self,field:&Field,here:&Member,there:&Member)->Result<(i64,i64)> {
        super::obsidian_base_dynamic::require_manual_membership(&self.connection,field.source)?;
        super::obsidian_base_dynamic::require_manual_membership(&self.connection,field.target)?;
        let found:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM relation_fields f JOIN collections c
            ON c.id=CASE ?1 WHEN 1 THEN f.source_collection ELSE f.target_collection END WHERE f.id=?2
            AND f.source_collection=?3 AND f.target_collection=?4 AND f.name=?5 COLLATE BINARY AND c.name=?6 COLLATE BINARY)",
            params![field.reverse,field.id,field.source,field.target,field.name,field.other],|r|r.get(0))?;
        let (from,to)=if field.reverse {(there,here)} else {(here,there)};
        if !found || from.id==to.id || member(&self.connection,field.source,from.id)?!=*from ||
            member(&self.connection,field.target,to.id)?!=*to {return Err("欄位或筆記已變更，請重新選取。".into());}
        Ok((from.id,to.id))
    }

    pub(crate) fn edit_field_value(&self,field:&Field,here:&Member,there:&Member,add:bool)->Result<()> {
        let tx=rusqlite::Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        let (from,to)=self.check_field_pair(field,here,there)?;
        let changed=if add {
            tx.execute("INSERT INTO relation_field_values(field_id,source_collection,target_collection,source_note,target_note)
                VALUES(?,?,?,?,?) ON CONFLICT(field_id,source_note,target_note) DO NOTHING",params![field.id,field.source,field.target,from,to])?
        }else{
            tx.execute("DELETE FROM relation_field_values WHERE field_id=? AND source_note=? AND target_note=?",params![field.id,from,to])?
        };
        if changed!=1 {return Err("關聯值已變更，請重新選取。".into());}
        tx.commit()?; Ok(())
    }

    pub(crate) fn open_field_value(&self,field:&Field,here:&Member,there:&Member)->Result<Note> {
        let tx=self.connection.unchecked_transaction()?;
        let (from,to)=self.check_field_pair(field,here,there)?;
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM relation_field_values WHERE field_id=? AND source_note=? AND target_note=?)",
            params![field.id,from,to],|r|r.get::<_,bool>(0))? {return Err("關聯值已移除。".into());}
        let note=self.note(there.id)?.ok_or("找不到筆記。")?;
        tx.commit()?; Ok(note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn dynamic_base_is_not_a_relation_field_endpoint()->Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let root=db.import_vault("fixture",vec![crate::archive::ImportedPage {
            parts:vec!["A".into(),"shot.md".into()],title:"Shot".into(),body:"---\napproved: true\n---\nreal".into()
        }])?;
        let raw=b"filters: 'note.approved == true'\nformulas: {flag: 'note.approved == true'}\nviews: [{type: table, name: Active, order: [file.path, approved]}]";
        let (_,saved)=db.import_obsidian_base_options(root,"active.base",raw.to_vec(),true,true)?;
        let dynamic=saved[0].0;
        let row=db.collection_rows(dynamic)?[0].0.id;
        let ordinary=db.create_collection("Manual")?;
        db.add_to_collection(ordinary,row,"ready")?;
        let available=db.relation_collections(row)?;
        assert_eq!(available.len(),1);
        assert_eq!(available[0].0,ordinary);
        let origin=&available[0].2;
        assert!(db.relation_target_collections()?.iter().all(|(id,_)|*id!=dynamic));
        let dynamic_name=db.collections()?.into_iter().find(|e|e.0==dynamic).unwrap().1;
        for (s,t) in [((ordinary,"Manual"),(dynamic,dynamic_name.as_str())),
                      ((dynamic,dynamic_name.as_str()),(ordinary,"Manual"))] {
            assert!(db.create_relation_field(origin,s,t,"bad").is_err());
        }
        db.create_relation_field(origin,(ordinary,"Manual"),(ordinary,"Manual"),"local")?;
        assert_eq!(db.editable_relation_fields(row)?.len(),2); // both roles in the ordinary collection
        Ok(())
    }

    #[test]
    fn reciprocal_field_edits_use_one_value_across_reopen_and_backup()->Result<()> {
        let dir=super::super::tests::data_dir();
        let path=dir.join("workspace.sqlite3");
        let mut db=Notebook::open(&path,false)?;
        let source=db.create_collection("Scenes")?;
        let target=db.create_collection("Shots")?;
        let a=db.add(None,"same","unique source")?;
        let b=db.add(None,"same","chosen target")?;
        let decoy=db.add(None,"same","other target")?;
        db.add_to_collection(source,a,"ready")?;
        for id in [b,decoy] {db.add_to_collection(target,id,"available")?;}
        db.set_relation(a,"References",decoy,true)?;
        let origin=db.relation_collections(a)?.remove(0).2;
        db.create_relation_field(&origin,(source,"Scenes"),(target,"Shots"),"References")?;
        assert!(db.create_relation_field(&origin,(source,"Scenes"),(target,"Shots"),"references").is_err());
        let forward=db.editable_relation_fields(a)?.remove(0).0;
        let pick=db.field_candidates(&forward,&origin,false)?.into_iter().find(|m|m.id==b).unwrap();
        db.edit_field_value(&forward,&origin,&pick,true)?;
        let (reverse,other)=db.editable_relation_fields(b)?.remove(0);
        assert!(reverse.reverse);
        assert_eq!(db.open_field_value(&reverse,&other,&origin)?.body,"unique source");
        assert!(db.edit_field_value(&reverse,&other,&origin,true).is_err());
        db.edit_field_value(&reverse,&other,&origin,false)?;
        assert!(db.open_field_value(&forward,&origin,&pick).is_err());
        assert!(db.field_candidates(&forward,&origin,true)?.is_empty());
        db.edit_field_value(&reverse,&other,&origin,true)?;
        db.trash(b)?;
        assert!(db.field_candidates(&forward,&origin,true)?.is_empty());
        assert!(db.edit_field_value(&forward,&origin,&pick,false).is_err());
        db.restore(b)?;
        drop(db);
        let db=Notebook::open(&path,false)?;
        assert_eq!(db.open_field_value(&forward,&origin,&pick)?.body,"chosen target");
        let backup=dir.join("backup.sqlite3");
        db.backup(&backup)?;
        drop(db);
        let isolated=dir.join("isolated.sqlite3");fs::copy(backup,&isolated)?;
        let db=Notebook::open(&isolated,false)?;
        assert_eq!(db.open_field_value(&reverse,&other,&origin)?.row.id,a);
        assert_eq!(db.relations(a)?[0].1.id,decoy);
        // Removing collection membership clears the scoped value but preserves both pages and the legacy edge.
        assert!(db.remove_collection_member(target,b,"available",Some(&[])).is_err());
        let outgoing = db.member_field_edges(target,b)?;
        assert_eq!(outgoing, [(forward.id,a,b)]);
        db.remove_collection_member(target,b,"available",Some(&outgoing))?;
        assert!(db.field_candidates(&forward,&origin,true)?.is_empty());
        assert!(db.open_field_value(&forward,&origin,&pick).is_err());
        assert_eq!(db.note(b)?.unwrap().body,"chosen target");
        drop(db);fs::remove_dir_all(dir)?;Ok(())
    }
}
