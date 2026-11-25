//! Local collections, status lanes and explicit directed note relations.
use super::{Notebook, Result, Row, insert, read_row};
use super::collection_field_rules::{CollectionRuleInput, apply_on_admission};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

#[path = "collection_templates.rs"]
mod collection_templates;
use collection_templates::checked_title;
pub(super) use collection_templates::{TEMPLATE_SCHEMA, TEMPLATE_RESOURCE_SCHEMA, TEMPLATE_CHILD_SCHEMA, TEMPLATE_NESTED_SCHEMA};
#[cfg(windows)]
pub(crate) use collection_templates::{ChildDraft, CollectionTemplate, TemplateApply, TemplateChild, TemplateField, TemplateSelection, validate_template_body};

pub(super) const SCHEMA: &str = "
CREATE TABLE collections (
 id INTEGER PRIMARY KEY, name TEXT NOT NULL COLLATE NOCASE UNIQUE
);
CREATE TABLE collection_notes (
 collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
 note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
 status TEXT NOT NULL,
 PRIMARY KEY(collection_id,note_id)
);
CREATE INDEX collection_notes_status ON collection_notes(collection_id,status,note_id);
CREATE TABLE note_relations (
 source_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
 name TEXT NOT NULL,
 target_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
 PRIMARY KEY(source_id,name,target_id), CHECK(source_id <> target_id)
);
CREATE INDEX note_relations_target ON note_relations(target_id,source_id);
";
const MAX_NAME: usize = 256;
pub(super) const MAX_ITEMS: i64 = 1000;
const MAX_COLLECTIONS: i64 = 128;
const MAX_RELATIONS: i64 = 1000;

fn checked_label(value: &str) -> Result<&str> {
    if value.contains(['\0','\n','\r','\t']) { return Err("名稱或狀態不能包含控制字元。".into()); }
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_NAME { return Err("名稱或狀態需有內容且最多 256 位元組。".into()); }
    Ok(value)
}

#[derive(Clone, Copy)]
pub(crate) struct RelationMember<'a> {
    pub note: i64,
    pub status: &'a str,
}

#[derive(Eq, PartialEq)]
pub(crate) struct InspectedCollectionLink {
    pub name: String,
    pub title: String,
    pub target: i64,
    pub current_status: Option<String>,
    pub trash: bool,
    token: String,
}

fn require_member(db: &Connection, collection: i64, member: RelationMember<'_>) -> Result<()> {
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM collection_notes c JOIN notes n ON n.id=c.note_id
         WHERE c.collection_id=?1 AND c.note_id=?2 AND c.status=?3 AND n.trash_root IS NULL)",
        params![collection, member.note, member.status], |row| row.get(0),
    )?;
    if !active { return Err("資料列已移出集合、移到垃圾桶或變更狀態。請重新整理。".into()); }
    Ok(())
}

fn write_relation(tx: &Transaction<'_>, source: i64, name: &str, target: i64, present: bool) -> Result<bool> {
    let args = params![source, name, target];
    if present {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM note_relations WHERE source_id=?1 AND name=?2 AND target_id=?3)",
            args, |row| row.get(0),
        )?;
        if exists { return Ok(false); }
        let count: i64 = tx.query_row("SELECT count(*) FROM note_relations WHERE source_id=?1", [source], |row| row.get(0))?;
        if count >= MAX_RELATIONS { return Err("每篇筆記最多 1000 條關聯。".into()); }
        tx.execute("INSERT INTO note_relations(source_id,name,target_id) VALUES(?1,?2,?3)", args)?;
    } else if tx.execute("DELETE FROM note_relations WHERE source_id=?1 AND name=?2 AND target_id=?3", args)? != 1 {
        return Err("關聯不存在，請重新載入。".into());
    }
    Ok(true)
}

fn read_collection_links(
    db: &Connection, collection: i64, source: RelationMember<'_>, exact: Option<&InspectedCollectionLink>,
) -> Result<Vec<InspectedCollectionLink>> {
    require_member(db, collection, source)?;
    let mut query = db.prepare_cached(
        "SELECT r.name,n.title,n.id,c.status,n.trash_root IS NOT NULL,i.token
         FROM note_relations r JOIN notes n ON n.id=r.target_id
         JOIN note_link_identities i ON i.note_id=n.id
         LEFT JOIN collection_notes c ON c.collection_id=?1 AND c.note_id=n.id
         WHERE r.source_id=?2 AND (?3 IS NULL OR (r.name=?3 AND r.target_id=?4))
         ORDER BY r.name COLLATE BINARY,n.title COLLATE BINARY,n.id LIMIT ?5",
    )?;
    let rows = query.query_map(params![
        collection, source.note, exact.map(|edge| edge.name.as_str()), exact.map(|edge| edge.target), MAX_RELATIONS + 1,
    ], |row| Ok(InspectedCollectionLink {
        name: row.get(0)?, title: row.get(1)?, target: row.get(2)?,
        current_status: row.get(3)?, trash: row.get(4)?, token: row.get(5)?,
    }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() > MAX_RELATIONS as usize { return Err("關聯超過可顯示上限。".into()); }
    Ok(rows)
}

fn require_inspected_link(
    db: &Connection, collection: i64, source: RelationMember<'_>, inspected: &InspectedCollectionLink,
) -> Result<()> {
    if read_collection_links(db, collection, source, Some(inspected))?.first() != Some(inspected) {
        return Err("關聯、目標或目標在目前集合的狀態已變更，請重新整理。".into());
    }
    Ok(())
}

pub(crate) enum CollectionEntry<'a> {
    Existing(i64),
    New(&'a str),
}

pub(super) fn insert_collection(tx: &Transaction<'_>, name: &str) -> Result<i64> {
    let name = checked_label(name)?;
    let count: i64 = tx.query_row("SELECT count(*) FROM collections", [], |r| r.get(0))?;
    if count >= MAX_COLLECTIONS { return Err("集合已達 128 個上限。".into()); }
    tx.execute("INSERT INTO collections(name) VALUES(?1)", [name])?;
    Ok(tx.last_insert_rowid())
}

impl Notebook {
    pub fn create_collection(&self, name: &str) -> Result<i64> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let id = insert_collection(&tx, name)?;
        tx.commit()?;
        Ok(id)
    }
    pub fn collections(&self) -> Result<Vec<(i64, String)>> {
        let mut s = self.connection.prepare("SELECT id,name FROM collections ORDER BY name COLLATE NOCASE,id")?;
        Ok(s.query_map([], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }
    pub fn add_to_collection(&self, collection_id: i64, note_id: i64, status: &str) -> Result<()> {
        self.add_collection_entry(collection_id, CollectionEntry::Existing(note_id), status, None)?;
        Ok(())
    }

    pub(crate) fn add_collection_entry(
        &self, collection: i64, entry: CollectionEntry<'_>, status: &str,
        input: Option<CollectionRuleInput<'_>>,
    ) -> Result<i64> {
        let status = checked_label(status)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let (exists, count): (bool, i64) = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE id=?1),
                    (SELECT count(*) FROM collection_notes WHERE collection_id=?1)",
            [collection], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if !exists { return Err("找不到集合。".into()); }
        if count >= MAX_ITEMS { return Err("集合最多 1000 篇筆記，含垃圾桶中的筆記。".into()); }
        let note = match entry {
            CollectionEntry::Existing(note) => {
                let (active, member): (bool, bool) = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM notes WHERE id=?2 AND trash_root IS NULL),
                            EXISTS(SELECT 1 FROM collection_notes WHERE collection_id=?1 AND note_id=?2)",
                    params![collection,note], |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                if !active { return Err("找不到可使用的筆記。".into()); }
                if member { return Err("筆記已在此集合，請重新載入。".into()); }
                note
            }
            CollectionEntry::New(title) => insert(&tx, None, checked_title(title)?, "")?,
        };
        tx.execute("INSERT INTO collection_notes(collection_id,note_id,status) VALUES(?1,?2,?3)",
                   params![collection,note,status])?;
        apply_on_admission(&tx, collection, note, input)?;
        tx.commit()?;
        Ok(note)
    }
    /// Update only if the active member still has the displayed status.
    pub fn move_collection_note(&self, collection_id: i64, note_id: i64, expected_status: &str, new_status: &str) -> Result<()> {
        let expected_status = checked_label(expected_status)?;
        let new_status = checked_label(new_status)?;
        let changed = self.connection.execute(
            "UPDATE collection_notes SET status=?4 WHERE collection_id=?1 AND note_id=?2 AND status=?3
             AND EXISTS(SELECT 1 FROM notes WHERE id=?2 AND trash_root IS NULL)",
            params![collection_id,note_id,expected_status,new_status],
        )?;
        if changed != 1 { return Err("筆記已移出集合、移到垃圾桶，或狀態已變更；請重新載入。".into()); }
        Ok(())
    }
    pub fn remove_from_collection(&self, collection_id: i64, note_id: i64, expected_status: &str) -> Result<()> {
        let status = checked_label(expected_status)?;
        let changed = self.connection.execute(
            "DELETE FROM collection_notes WHERE collection_id=?1 AND note_id=?2 AND status=?3
             AND EXISTS(SELECT 1 FROM notes WHERE id=?2 AND trash_root IS NULL)",
            params![collection_id,note_id,status],
        )?;
        if changed != 1 { return Err("筆記已移出集合、移到垃圾桶，或狀態已變更；請重新載入。".into()); }
        Ok(())
    }
    /// Explicit member status; rows can be grouped into lanes by callers.
    pub fn collection_rows(&self, collection_id: i64) -> Result<Vec<(Row,String)>> {
        let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM collections WHERE id=?1)", [collection_id], |r| r.get(0))?;
        if !exists { return Err("找不到集合。".into()); }
        let mut s = self.connection.prepare(
            "SELECT n.id,n.parent,n.title,n.pinned,n.trash_root,c.status FROM collection_notes c
             JOIN notes n ON n.id=c.note_id WHERE c.collection_id=?1 AND n.trash_root IS NULL
             ORDER BY c.status COLLATE BINARY,n.title COLLATE BINARY,n.id LIMIT 1001",
        )?;
        let rows = s.query_map([collection_id], |r| Ok((read_row(r)?,r.get(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > MAX_ITEMS as usize { return Err("集合項目超過可顯示上限。".into()); }
        Ok(rows)
    }
    /// Directed explicit-ID relation; both endpoints must remain active.
    pub fn set_relation(&self, source_id: i64, name: &str, target_id: i64, present: bool) -> Result<()> {
        let name = checked_label(name)?;
        if source_id == target_id { return Err("不能將筆記關聯到自身。".into()); }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let active: i64 = tx.query_row("SELECT count(*) FROM notes WHERE id IN (?1,?2) AND trash_root IS NULL", params![source_id,target_id], |r| r.get(0))?;
        if active != 2 { return Err("來源或目標筆記不存在或已移到垃圾桶。".into()); }
        write_relation(&tx, source_id, name, target_id, present)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn set_collection_relation(
        &self, collection: i64, source: RelationMember<'_>, name: &str, target: RelationMember<'_>, present: bool,
    ) -> Result<()> {
        self.write_collection_relation(collection, source, name, collection, target, present)
    }

    /// The target collection constrains selection. The persisted link follows the target note.
    pub(crate) fn add_collection_relation_between(
        &self, collection: i64, source: RelationMember<'_>, name: &str,
        target_collection: i64, target: RelationMember<'_>,
    ) -> Result<()> {
        self.write_collection_relation(collection, source, name, target_collection, target, true)
    }

    fn write_collection_relation(
        &self, collection: i64, source: RelationMember<'_>, name: &str,
        target_collection: i64, target: RelationMember<'_>, present: bool,
    ) -> Result<()> {
        let name = checked_label(name)?;
        if source.note == target.note { return Err("不能將筆記關聯到自身。".into()); }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        require_member(&tx, collection, source)?;
        require_member(&tx, target_collection, target)?;
        if !write_relation(&tx, source.note, name, target.note, present)? {
            return Err("這條關聯已存在，請重新整理。".into());
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn inspect_collection_links(
        &self, collection: i64, source: RelationMember<'_>,
    ) -> Result<Vec<InspectedCollectionLink>> {
        let tx = self.connection.unchecked_transaction()?;
        let links = read_collection_links(&tx, collection, source, None)?;
        tx.commit()?;
        Ok(links)
    }

    pub(crate) fn open_inspected_collection_link(
        &self, collection: i64, source: RelationMember<'_>, link: &InspectedCollectionLink,
    ) -> Result<i64> {
        let tx = self.connection.unchecked_transaction()?;
        require_inspected_link(&tx, collection, source, link)?;
        if link.trash { return Err("關聯目標在垃圾桶，請先還原。".into()); }
        tx.commit()?;
        Ok(link.target)
    }

    pub(crate) fn remove_inspected_collection_link(
        &self, collection: i64, source: RelationMember<'_>, link: &InspectedCollectionLink,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        require_inspected_link(&tx, collection, source, link)?;
        write_relation(&tx, source.note, &link.name, link.target, false)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn collection_relation_totals(
        &self, collection: i64, source: RelationMember<'_>, name: &str, property: Option<&str>,
    ) -> Result<(usize, usize, Option<f64>)> {
        let tx = self.connection.unchecked_transaction()?;
        let name = checked_label(name)?;
        let local = self.collection_relations(collection, source)?
            .iter().filter(|(label, _, _)| label == name).count();
        let total = self.rollup_count(source.note, name)?;
        let sum = property.map(|property| self.rollup_sum(source.note, name, property)).transpose()?;
        tx.commit()?;
        Ok((local, total, sum))
    }

    /// Show the existing same-collection relation subset.
    pub(crate) fn collection_relations(
        &self, collection: i64, source: RelationMember<'_>,
    ) -> Result<Vec<(String, Row, String)>> {
        require_member(&self.connection, collection, source)?;
        let mut stmt = self.connection.prepare(
            "SELECT n.id,n.parent,n.title,n.pinned,n.trash_root,r.name,c.status
             FROM note_relations r JOIN notes n ON n.id=r.target_id
             JOIN collection_notes c ON c.note_id=n.id AND c.collection_id=?1
             WHERE r.source_id=?2 AND n.trash_root IS NULL
             ORDER BY r.name COLLATE BINARY,c.status COLLATE BINARY,n.title COLLATE BINARY,n.id LIMIT 1001",
        )?;
        let rows = stmt.query_map(params![collection, source.note], |row| Ok((row.get(5)?, read_row(row)?, row.get(6)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > MAX_RELATIONS as usize { return Err("關聯超過可顯示上限。".into()); }
        Ok(rows)
    }

    pub fn relations(&self, source_id: i64) -> Result<Vec<(String,Row)>> {
        let active = self.connection.query_row("SELECT 1 FROM notes WHERE id=?1 AND trash_root IS NULL", [source_id], |r| r.get::<_,i64>(0)).optional()?;
        if active.is_none() { return Err("找不到可使用的來源筆記。".into()); }
        let mut s = self.connection.prepare(
            "SELECT n.id,n.parent,n.title,n.pinned,n.trash_root,r.name FROM note_relations r
             JOIN notes n ON n.id=r.target_id WHERE r.source_id=?1 AND n.trash_root IS NULL
             ORDER BY r.name COLLATE BINARY,n.title COLLATE BINARY,n.id LIMIT 1001",
        )?;
        let rows = s.query_map([source_id], |r| Ok((r.get(5)?,read_row(r)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > MAX_RELATIONS as usize { return Err("關聯超過可顯示上限。".into()); }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn collection_relation_workflow_keeps_scoped_targets_and_shared_live_rollups_after_reopen() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!("paperroom-member-relations-{}-{}.db",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        let mut db = Notebook::open(&path, false)?;
        let collection = db.create_collection("分鏡")?;
        let other_collection = db.create_collection("外部")?;
        let source = db.add(None, "主要鏡頭", "")?;
        let folder_a = db.add(None, "棚內", "")?;
        let folder_b = db.add(None, "棚外", "")?;
        let target = db.add(Some(folder_a), "鏡頭", "")?;
        let external = db.add(Some(folder_b), "鏡頭", "")?;
        let member = RelationMember { note: source, status: "本期" };
        let in_collection = RelationMember { note: target, status: "下期" };
        let outside = RelationMember { note: external, status: "下期" };
        db.add_to_collection(collection, source, member.status)?;
        db.add_to_collection(collection, target, in_collection.status)?;
        db.add_to_collection(other_collection, external, outside.status)?;
        db.compare_set_property(target, "工時", None, Some("2.50"))?;
        db.compare_set_property(external, "工時", None, Some("3"))?;
        db.set_relation(source, "子項", external, true)?;
        db.set_relation(source, "子項", external, true)?;
        assert!(db.collection_relations(collection, member)?.is_empty());
        assert_eq!(db.rollup_count(source, "子項")?, 1);
        assert!(db.set_collection_relation(collection, member, "子項", outside, true).is_err());
        assert!(db.set_collection_relation(collection, member, "子項", member, true).is_err());
        let saved = db.create_collection_view(
            collection, "只顯示本期", crate::store::collection_views::CollectionLayout::Table, "本期", "",
        )?;
        assert_eq!(db.filtered_collection_rows(collection, Some(&saved))?.len(), 1);
        db.set_collection_relation(collection, member, " 子項 ", in_collection, true)?;
        assert!(db.set_collection_relation(collection, member, "子項", in_collection, true).is_err());
        assert_eq!(db.collection_relations(collection, member)?[0].1.id, target);
        assert_eq!(db.rollup_count(source, "子項")?, 2);
        assert_eq!(db.evaluate_note_formula(source, r#"relation_sum("子項","工時")"#)?, "5.5");

        drop(db);
        let db = Notebook::open(&path, false)?;
        let edges = db.collection_relations(collection, member)?;
        assert_eq!((edges[0].0.as_str(), edges[0].1.id, edges[0].2.as_str()), ("子項", target, "下期"));
        db.move_collection_note(collection, target, "下期", "本期")?;
        assert!(db.set_collection_relation(collection, member, "子項", in_collection, false).is_err());
        assert_eq!(db.rollup_count(source, "子項")?, 2);
        let in_collection = RelationMember { note: target, status: "本期" };
        db.remove_from_collection(collection, target, in_collection.status)?;
        assert!(db.collection_relations(collection, member)?.is_empty());
        assert!(db.set_collection_relation(collection, member, "子項", in_collection, false).is_err());
        assert_eq!(db.rollup_count(source, "子項")?, 2);
        db.add_to_collection(collection, target, in_collection.status)?;
        db.trash(target)?;
        assert!(db.collection_relations(collection, member)?.is_empty());
        assert!(db.set_collection_relation(collection, member, "子項", in_collection, false).is_err());
        assert_eq!(db.rollup_count(source, "子項")?, 1);
        let mut db = db;
        db.restore(target)?;
        db.set_collection_relation(collection, member, "子項", in_collection, false)?;
        assert!(db.set_collection_relation(collection, member, "子項", in_collection, false).is_err());
        assert!(db.collection_relations(collection, member)?.is_empty());
        assert_eq!(db.relations(source)?[0].1.id, external);
        assert_eq!(db.evaluate_note_formula(source, r#"relation_sum("子項","工時")"#)?, "3");
        db.move_collection_note(collection, source, member.status, "已完成")?;
        assert!(db.collection_relations(collection, member).is_err());
        drop(db);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn collection_lanes_relations_and_trash() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let a = db.add(None,"甲","")?; let b = db.add(None,"乙","")?;
        let c = db.create_collection("專案")?;
        db.add_to_collection(c,a,"待辦")?; db.add_to_collection(c,b,"完成")?;
        db.set_relation(a,"依賴",b,true)?;
        assert_eq!(db.relations(a)?[0].1.id,b);
        assert_eq!(db.collection_rows(c)?.len(),2);
        db.trash(b)?;
        assert_eq!(db.collection_rows(c)?.len(),1);
        assert!(db.relations(a)?.is_empty());
        assert!(db.move_collection_note(c,b,"完成","待辦").is_err());
        db.restore(b)?;
        assert_eq!(db.relations(a)?[0].1.id,b);
        assert_eq!(db.collection_rows(c)?.len(),2);
        assert!(db.set_relation(a,"錯誤",a,true).is_err());
        Ok(())
    }
    #[test]
    fn stale_moves_and_deletes_are_rejected() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir = std::env::temp_dir().join(format!("paperroom-collections-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&dir)?;
        let path = dir.join("notes.sqlite3");
        let mut a = Notebook::open(&path,false)?;
        let b = Notebook::open(&path,false)?;
        let note = a.add(None,"頁","")?; let collection = a.create_collection("任務")?;
        a.add_to_collection(collection,note,"待辦")?;
        b.move_collection_note(collection,note,"待辦","進行中")?;
        assert!(a.move_collection_note(collection,note,"待辦","完成").is_err());
        assert!(a.remove_from_collection(collection,note,"待辦").is_err());
        assert_eq!(a.collection_rows(collection)?[0].1,"進行中");
        a.trash(note)?;
        assert!(b.remove_from_collection(collection,note,"進行中").is_err());
        a.restore(note)?;
        assert_eq!(a.collection_rows(collection)?[0].1,"進行中");
        drop(b); drop(a); std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
