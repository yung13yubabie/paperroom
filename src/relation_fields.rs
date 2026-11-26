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

impl Notebook {
    pub(crate) fn save_field_rollup(&self, field:&Field, origin:&Member, key:Option<&str>, name:&str)->Result<()> {
        let tx=rusqlite::Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        if !self.editable_relation_fields(origin.id)?.iter().any(|(f,m)| f==field && m==origin) {
            return Err("關聯屬性或筆記已變更，請重新選取。".into());
        }
        let collection=if field.reverse {field.target} else {field.source};
        let direction=if field.reverse {"in"} else {"out"};
        let expression=if let Some(key)=key {
            self.require_rollup_numeric_key(key)?;
            let quoted=key.replace('\\',"\\\\").replace('"', r#"\""#);
            format!(r#"field_sum("{}","{}","{}")"#,field.id,direction,quoted)
        } else {
            format!(r#"field_count("{}","{}")"#,field.id,direction)
        };
        self.add_collection_formula(collection,name,&expression)?;
        tx.commit()?; Ok(())
    }

    fn require_rollup_numeric_key(&self,key:&str)->Result<()> {
        if !self.connection.prepare_cached(
            "SELECT 1 FROM property_definitions WHERE key=?1 AND kind='number'")?.exists([key])? {
            return Err(format!("請先將「{key}」定義為數字屬性。").into());
        }
        Ok(())
    }

    // Invoked inside the caller's read transaction, including scans of saved views.
    pub(super) fn relation_field_rollup(&self, note:i64, field_id:&str, role:&str, key:Option<&str>)->Result<f64> {
        let field = field_id.parse::<i64>().ok().filter(|n| *n>0 && n.to_string()==field_id)
            .ok_or("關聯屬性編號必須是正整數。")?;
        let (local,here,there) = match role {
            "in" => ("target_collection","target_note","source_note"),
            "out" => ("source_collection","source_note","target_note"),
            _ => return Err("關聯方向必須為 in 或 out。".into()),
        };
        let active=self.connection.prepare_cached(&format!(
            "SELECT 1 FROM relation_fields f JOIN collection_notes m ON m.collection_id=f.{local}
             JOIN notes n ON n.id=m.note_id WHERE f.id=?1 AND n.id=?2 AND n.trash_root IS NULL"))?
            .exists(params![field,note])?;
        if !active {return Err(format!("找不到適用於筆記 #{note} 的關聯屬性 #{field}。").into());}
        let from=format!("FROM relation_field_values v JOIN notes n ON n.id=v.{there}");
        let condition=format!("WHERE v.field_id=?1 AND v.{here}=?2 AND n.trash_root IS NULL");
        let Some(key)=key else {
            let count:i64=self.connection.prepare_cached(&format!("SELECT count(*) {from} {condition}"))?
                .query_row(params![field,note],|r|r.get(0))?;
            return Ok(count as f64);
        };
        self.require_rollup_numeric_key(key)?;
        let mut query=self.connection.prepare_cached(&format!(
            "SELECT n.id,p.value {from} JOIN note_properties p ON p.note_id=n.id AND p.key=?3 {condition} ORDER BY n.id"))?;
        let mut values=query.query(params![field,note,key])?;
        let mut sum=0.0;
        while let Some(row)=values.next()? {
            let id:i64=row.get(0)?;
            let raw:String=row.get(1)?;
            let number=super::typed_properties::PropertyType::Number.normalized(&raw)
                .and_then(|s|s.parse::<f64>().ok()).filter(|n|n.is_finite())
                .ok_or_else(||format!("請修正筆記 #{id} 的數字屬性「{key}」。"))?;
            sum+=number;
            if !sum.is_finite() {return Err("數字加總超出公式範圍。".into());}
        }
        Ok(sum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn field_rollup_drives_saved_numeric_filter_sort_and_isolated_backup()->Result<()> {
        use super::super::collection_views::{CollectionLayout,FilterKind,FilterOperator,FilterRule,SortRule,SortSource,ViewRules};
        use super::super::typed_properties::PropertyType;
        let dir=super::super::tests::data_dir();
        let path=dir.join("original.sqlite3");
        let mut db=Notebook::open(&path,false)?;
        let col=db.create_collection("source")?;let peers=db.create_collection("target")?;
        let a=db.add(None,"A","A")?;let b=db.add(None,"B","B")?;let empty=db.add(None,"Empty","")?;
        let x=db.add(None,"same","X")?;let y=db.add(None,"same","Y")?;
        for id in [a,b,empty]{db.add_to_collection(col,id,"ready")?;}
        for id in [x,y]{db.add_to_collection(peers,id,"ready")?;}
        let origin=db.relation_collections(a)?.remove(0).2;
        db.create_relation_field(&origin,(col,"source"),(peers,"target"),"鏡頭")?;
        let (field,one)=db.editable_relation_fields(a)?.remove(0);
        let (_,two)=db.editable_relation_fields(b)?.remove(0);
        let (reverse,px)=db.editable_relation_fields(x)?.remove(0);
        let (_,py)=db.editable_relation_fields(y)?.remove(0);
        db.edit_field_value(&field,&one,&px,true)?;
        db.edit_field_value(&field,&two,&py,true)?;
        db.set_relation(a,"鏡頭",y,true)?; // same name belongs to the older, independent data model
        let key=r#"工時"\鏡頭"#;
        db.compare_set_property_type(key,None,Some(PropertyType::Number))?;
        db.save_field_rollup(&field,&one,Some(key),"Total")?;
        db.save_field_rollup(&reverse,&px,None,"Sources")?;
        let expr=db.collection_formulas(col)?[0].1.clone();
        let count=db.collection_formulas(peers)?[0].1.clone();
        assert_eq!(db.evaluate_note_formula(x,&count)?,"1");
        db.save_field_rollup(&reverse,&px,Some(key),"ReverseTotal")?;
        let reverse_expr=db.collection_formulas(peers)?.into_iter().find(|(n,_)|n=="ReverseTotal").unwrap().1;
        db.compare_set_property(a,key,None,Some("-4"))?;
        assert_eq!(db.evaluate_note_formula(x,&reverse_expr)?,"-4");
        for invalid in [r#"field_count("01","out")"#,r#"field_count("9223372036854775807","in")"#,
                        r#"field_count("1","reverse")"#] {
            assert!(db.evaluate_note_formula(a,invalid).is_err());
        }
        assert_eq!(db.evaluate_note_formula(a,&expr)?,"0");
        assert_eq!(db.evaluate_note_formula(empty,&expr)?,"0");
        assert!(db.evaluate_note_formula(x,&expr).is_err());
        assert!(db.save_field_rollup(&field,&one,Some("missing"),"bad").is_err());
        assert!(db.save_field_rollup(&field,&one,None,"Total").is_err());
        db.compare_set_property(x,key,None,Some("12"))?;db.compare_set_property(y,key,None,Some("8"))?;
        let view=db.create_collection_view(col,"Totals",CollectionLayout::Table,"","")?;
        let rules=ViewRules {
            filters:vec![FilterRule{key:"Total".into(),kind:FilterKind::Formula(Some(PropertyType::Number)),
                operator:FilterOperator::Greater,value:"0".into(),argument:None}],
            sorts:vec![SortRule{key:"Total".into(),source:SortSource::Formula(Some(PropertyType::Number)),descending:true}],
            ..Default::default()
        };
        db.set_collection_view_rules(col,&view,rules.clone())?;
        let rows=|db:&Notebook|->Result<Vec<i64>> {
            let saved=db.collection_views(col)?.remove(0);
            assert_eq!(saved.rules(),rules);
            Ok(db.presented_collection_lanes(col,Some(&saved))?.into_iter().flat_map(|(_,rows)|rows.into_iter().map(|r|r.id)).collect())
        };
        assert_eq!(rows(&db)?,[a,b]);
        db.compare_set_property(x,key,Some("12"),Some("3"))?;
        assert_eq!(rows(&db)?,[b,a]);
        db.trash(y)?;
        assert_eq!(rows(&db)?,[a]);
        db.restore(y)?;
        db.connection.execute("UPDATE note_properties SET value='invalid' WHERE note_id=?1 AND key=?2",params![x,key])?;
        assert!(db.evaluate_note_formula(a,&expr).unwrap_err().to_string().contains(&format!("#{x}")));
        db.compare_set_property(x,key,Some("invalid"),Some("0"))?;
        assert_eq!(rows(&db)?,[b]);
        drop(db);
        let db=Notebook::open(&path,false)?;
        assert_eq!(rows(&db)?,[b]);
        let backup=dir.join("backup.sqlite3");db.backup(&backup)?;drop(db);
        let copy=dir.join("restored.sqlite3");fs::copy(backup,&copy)?;
        let db=Notebook::open(&copy,false)?;
        assert_eq!(db.evaluate_note_formula(x,&count)?,"1");
        assert_eq!(rows(&db)?,[b]);
        drop(db);fs::remove_dir_all(dir)?;Ok(())
    }

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
