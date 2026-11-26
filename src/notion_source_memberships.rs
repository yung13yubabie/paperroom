//! Reverse the explicit local source relation through active, ordinary memberships.
use super::SOURCE_RELATION;
use crate::store::{Notebook,Note,Result};
use rusqlite::{OptionalExtension,params};

#[derive(PartialEq,Eq)]
pub(crate) struct PageIdentity {
    id:i64, token:String, root:i64, root_token:String, part:String, path:String,
}

#[derive(PartialEq,Eq)]
pub(crate) struct SourceMembership {
    pub collection:i64, pub collection_title:String, pub row:i64, pub title:String,
    status:String, token:String,
}

impl SourceMembership {
    pub(crate) fn label(&self)->String {
        format!("{}　[#{}]　｜ {}　｜ {}　[#{}]",self.collection_title,self.collection,self.status,self.title,self.row)
    }
}

const MEMBERS:&str="FROM note_relations r JOIN collection_notes m ON m.note_id=r.source_id
    JOIN collections c ON c.id=m.collection_id JOIN notes n ON n.id=r.source_id
    JOIN note_link_identities i ON i.note_id=n.id
    WHERE r.target_id=?1 AND r.name=?2 AND n.trash_root IS NULL
    AND NOT EXISTS(SELECT 1 FROM obsidian_base_dynamic d WHERE d.collection_id=c.id)";
const AMBIGUOUS:&str="EXISTS(SELECT 1 FROM note_relations other WHERE other.source_id=r.source_id
    AND other.name=r.name AND other.target_id<>r.target_id)";
const COLUMNS:&str="SELECT c.id,c.name,n.id,n.title,m.status,i.token";

fn read_membership(r:&rusqlite::Row<'_>)->rusqlite::Result<SourceMembership> {
    Ok(SourceMembership{collection:r.get(0)?,collection_title:r.get(1)?,row:r.get(2)?,title:r.get(3)?,status:r.get(4)?,token:r.get(5)?})
}

impl Notebook {
    pub(crate) fn notion_source_identity(&self,id:i64)->Result<Option<PageIdentity>> {
        Ok(self.connection.query_row("SELECT n.id,i.token,p.import_root,ri.token,p.archive_part,p.relative_path
            FROM notion_page_paths p JOIN notes n ON n.id=p.note_id JOIN note_link_identities i ON i.note_id=n.id
            JOIN notes root ON root.id=p.import_root JOIN note_link_identities ri ON ri.note_id=root.id
            WHERE n.id=?1 AND n.trash_root IS NULL AND root.trash_root IS NULL",[id],|r|Ok(PageIdentity{
                id:r.get(0)?,token:r.get(1)?,root:r.get(2)?,root_token:r.get(3)?,part:r.get(4)?,path:r.get(5)?,
            })).optional()?)
    }
    fn require_source_identity(&self,page:&PageIdentity)->Result<()> {
        if self.notion_source_identity(page.id)?.as_ref()!=Some(page) {
            return Err("Notion 來源頁或匯入批次已變更，請重新開啟。".into());
        }
        Ok(())
    }
    pub(crate) fn notion_membership_page(&self,page:&PageIdentity,offset:usize)->Result<(Vec<SourceMembership>,bool,i64)> {
        let tx=self.connection.unchecked_transaction()?;
        self.require_source_identity(page)?;
        let sql=format!("{COLUMNS} {MEMBERS} AND NOT {AMBIGUOUS}
            ORDER BY c.name COLLATE BINARY,c.id,n.title COLLATE BINARY,n.id LIMIT 41 OFFSET ?3");
        let mut hits=self.connection.prepare_cached(&sql)?.query_map(
            params![page.id,SOURCE_RELATION,i64::try_from(offset)?],read_membership)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let ambiguous=if offset==0 {
            self.connection.query_row(&format!("SELECT COUNT(DISTINCT r.source_id) {MEMBERS} AND {AMBIGUOUS}"),
                params![page.id,SOURCE_RELATION],|r|r.get(0))?
        }else{0};
        let more=hits.len()>40; hits.truncate(40);
        tx.commit()?; Ok((hits,more,ambiguous))
    }
    pub(crate) fn note_from_source_membership(&self,page:&PageIdentity,hit:&SourceMembership)->Result<Note> {
        self.read_source_membership(page,hit,|| self.note(hit.row)?.ok_or_else(||"找不到資料列筆記。".into()))
    }

    pub(crate) fn read_source_membership<T>(&self,page:&PageIdentity,hit:&SourceMembership,read:impl FnOnce()->Result<T>)->Result<T> {
        let tx=self.connection.unchecked_transaction()?;
        self.require_source_identity(page)?;
        let sql=format!("{COLUMNS} {MEMBERS} AND NOT {AMBIGUOUS} AND c.id=?3 AND n.id=?4");
        let found=self.connection.query_row(&sql,params![page.id,SOURCE_RELATION,hit.collection,hit.row],read_membership).optional()?;
        if found.as_ref()!=Some(hit) {return Err("資料列、集合成員資格或來源關聯已變更，請重新查詢。".into());}
        let result=read()?;
        tx.commit()?; Ok(result)
    }
}
