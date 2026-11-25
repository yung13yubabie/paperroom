//! Note properties retain their original text, including typed values. The parent module owns schema versioning.
use super::{Notebook, Result, Row, read_row};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

pub(super) const SCHEMA: &str = "
CREATE TABLE note_properties (
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (note_id, key)
);
CREATE INDEX note_properties_by_key_value ON note_properties(key, value, note_id);
";

pub const MAX_KEY_BYTES: usize = 256;
pub const MAX_VALUE_BYTES: usize = 4096;
pub(super) const MAX_PROPERTIES: i64 = 128;
const MAX_QUERY_BYTES: usize = 4096;
const MAX_ROWS: i64 = 1000;

pub fn key_checked(key: &str) -> Result<&str> {
    if key.contains(['\r', '\n', '\t']) {
        return Err("屬性名稱不得包含換行或定位字元。".into());
    }
    let key = key.trim();
    if key.is_empty() || key.len() > MAX_KEY_BYTES || key.contains('\0') {
        return Err("屬性名稱必須有內容，且不得超過 256 位元組或包含 NUL。".into());
    }
    Ok(key)
}

pub(super) fn write_value(tx: &Transaction<'_>, id: i64, key: &str, value: &str, new: bool) -> Result<()> {
    if value.trim().is_empty() {
        super::collection_field_rules::ensure_clear_allowed(tx, id, key)?;
    }
    if new {
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM note_properties WHERE note_id=?1", [id], |row| row.get(0),
        )?;
        if count >= MAX_PROPERTIES { return Err("每篇筆記最多 128 項屬性。".into()); }
    }
    tx.execute(
        "INSERT INTO note_properties(note_id,key,value) VALUES (?1,?2,?3)
         ON CONFLICT(note_id,key) DO UPDATE SET value=excluded.value",
        params![id,key,value],
    )?;
    Ok(())
}

pub(super) fn delete_value(tx: &Transaction<'_>, id: i64, key: &str) -> Result<()> {
    super::collection_field_rules::ensure_clear_allowed(tx, id, key)?;
    tx.execute("DELETE FROM note_properties WHERE note_id=?1 AND key=?2", params![id,key])?;
    Ok(())
}

impl Notebook {
    /// Set or remove without checking the old value. UI editors should use compare_set_property.
    #[cfg(test)]
    pub fn set_property(&self, id: i64, key: &str, value: Option<&str>) -> Result<()> {
        self.write_property(id, key, None, value)
    }

    /// Reject stale changes; None means no property and Some("") is an explicit empty value.
    pub fn compare_set_property(&self, id: i64, key: &str, expected: Option<&str>, value: Option<&str>) -> Result<()> {
        self.write_property(id, key, Some(expected), value)
    }

    fn write_property(&self, id: i64, key: &str, expected: Option<Option<&str>>, value: Option<&str>) -> Result<()> {
        let key = key_checked(key)?;
        for v in [expected.flatten(), value].into_iter().flatten() {
            if v.len() > MAX_VALUE_BYTES || v.contains('\0') {
                return Err("屬性值不得超過 4096 位元組或包含 NUL。".into());
            }
        }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM notes WHERE id=?1 AND trash_root IS NULL)",
            [id], |r| r.get(0),
        )?;
        if !active { return Err("找不到可編輯的筆記。".into()); }
        let current: Option<String> = tx.query_row(
            "SELECT value FROM note_properties WHERE note_id=?1 AND key=?2", params![id, key], |r| r.get(0),
        ).optional()?;
        if let Some(expected) = expected {
            if current.as_deref() != expected {
                return Err("屬性已由另一個視窗更改。請先複製修改，再重新載入。".into());
            }
        }
        if current.as_deref() == value { return Ok(()); }
        if let Some(value) = value {
            super::typed_properties::validate_write(&tx,key,value)?;
        }
        if super::typed_properties::lookup(&tx, key)? == Some(super::typed_properties::PropertyType::Date) {
            use super::collection_views::{CalendarDay, validate_collection_field_dates};
            validate_collection_field_dates(&tx, id, [(key, value.and_then(CalendarDay::parse)), ("", None)])?;
        }
        super::note_history_state::checkpoint(&tx, id)?;
        if let Some(value) = value {
            write_value(&tx,id,key,value,current.is_none())?;
        } else {
            delete_value(&tx, id, key)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Return the active note's explicitly stored properties in key order.
    pub fn properties(&self, id: i64) -> Result<Vec<(String, String)>> {
        let exists = self.connection.query_row(
            "SELECT 1 FROM notes WHERE id=?1 AND trash_root IS NULL", [id], |r| r.get::<_, i64>(0),
        ).optional()?;
        if exists.is_none() { return Err("找不到可檢視的筆記。".into()); }
        let mut s = self.connection.prepare(
            "SELECT key,value FROM note_properties WHERE note_id=?1 ORDER BY key COLLATE BINARY",
        )?;
        Ok(s.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Snapshot the raw values for one property column without normalizing legacy text.
    pub(crate) fn collection_property_values(
        &self, collection: i64, key: &str,
    ) -> Result<std::collections::HashMap<i64, String>> {
        let key = key_checked(key)?;
        let mut statement = self.connection.prepare(
            "SELECT c.note_id,p.value FROM collection_notes c
             JOIN note_properties p ON p.note_id=c.note_id
             JOIN notes n ON n.id=c.note_id
             WHERE c.collection_id=?1 AND p.key=?2 AND n.trash_root IS NULL",
        )?;
        Ok(statement.query_map(params![collection,key], |row| Ok((row.get(0)?,row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn toggle_collection_checkbox(
        &self, collection: i64, id: i64, status: &str, key: &str, expected: Option<&str>,
    ) -> Result<bool> {
        use super::typed_properties::{PropertyType, lookup};
        let key = key_checked(key)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let member: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM collection_notes c JOIN notes n ON n.id=c.note_id
                           WHERE c.collection_id=?1 AND c.note_id=?2 AND c.status=?3 AND n.trash_root IS NULL)",
            params![collection,id,status], |row| row.get(0),
        )?;
        if !member || lookup(&tx,key)? != Some(PropertyType::Checkbox) {
            return Err("資料列或核取方塊型別已更改。請重新整理檢視。".into());
        }
        let current: Option<String> = tx.query_row(
            "SELECT value FROM note_properties WHERE note_id=?1 AND key=?2",
            params![id,key], |row| row.get(0),
        ).optional()?;
        if current.as_deref() != expected {
            return Err("核取方塊已變更。請重新整理檢視。".into());
        }
        let checked = match current.as_deref() {
            None => true,
            Some(value) => match PropertyType::Checkbox.normalized(value).as_deref() {
                Some("true") => false,
                Some("false") => true,
                _ => return Err("這個值是舊文字。請先由筆記的屬性編輯器確認或修正。".into()),
            },
        };
        super::note_history_state::checkpoint(&tx, id)?;
        write_value(&tx,id,key,if checked { "true" } else { "false" }, current.is_none())?;
        tx.commit()?;
        Ok(checked)
    }

    /// Query explicit values for one key. `query` is a literal substring, not SQL or a wildcard.
    /// Results are limited to 1000 active notes and sorted by value, title, then stable note ID.
    pub fn property_rows(&self, key: &str, query: &str) -> Result<Vec<(Row, String)>> {
        let key = key_checked(key)?;
        if query.len() > MAX_QUERY_BYTES || query.contains('\0') {
            return Err("搜尋文字過長或包含 NUL。".into());
        }
        let mut s = self.connection.prepare(
            "SELECT n.id,n.parent,n.title,n.pinned,n.trash_root,p.value
             FROM note_properties p JOIN notes n ON n.id=p.note_id
             WHERE p.key=?1 AND n.trash_root IS NULL AND instr(p.value,?2)>0
             ORDER BY p.value COLLATE BINARY,n.title COLLATE BINARY,n.id LIMIT ?3",
        )?;
        Ok(s.query_map(params![key, query, MAX_ROWS], |r| Ok((read_row(r)?, r.get(5)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn database() -> Result<Notebook> {
        let db = Notebook::open(Path::new(":memory:"), false)?;
        Ok(db)
    }

    #[test]
    fn property_lifecycle_and_literal_filter() -> Result<()> {
        let mut db = database()?;
        let a = db.add(None, "甲", "")?;
        let b = db.add(None, "乙", "")?;
        db.set_property(a, " 狀態 ", Some("進行中 %_"))?;
        db.set_property(b, "狀態", Some("完成"))?;
        assert_eq!(db.properties(a)?, vec![("狀態".into(), "進行中 %_".into())]);
        let rows = db.property_rows("狀態", "%_")?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.id, a);
        db.trash(a)?;
        assert!(db.set_property(a, "狀態", Some("不應儲存")).is_err());
        assert!(db.properties(a).is_err());
        assert!(db.property_rows("狀態", "%_")?.is_empty());
        db.restore(a)?;
        assert_eq!(db.properties(a)?.len(), 1);
        db.set_property(a, "狀態", None)?;
        assert!(db.properties(a)?.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_invalid_and_excessive_properties() -> Result<()> {
        let mut db = database()?;
        let id = db.add(None, "頁面", "")?;
        assert!(db.set_property(id, " ", Some("x")).is_err());
        assert!(db.set_property(id, "bad\0key", Some("x")).is_err());
        for key in ["內\r部", "內\n部", "內\t部"] {
            assert!(db.set_property(id, key, Some("x")).is_err());
            assert!(db.compare_set_property(id, key, None, Some("x")).is_err());
        }
        assert!(db.set_property(id, "值", Some(&"x".repeat(MAX_VALUE_BYTES + 1))).is_err());
        for n in 0..MAX_PROPERTIES { db.set_property(id, &format!("k{n}"), Some("v"))?; }
        assert!(db.set_property(id, "extra", Some("v")).is_err());
        db.set_property(id, "k0", Some("更新"))?;
        assert!(db.set_property(-1, "k", None).is_err());
        Ok(())
    }
    #[test]
    fn stale_edits_deletes_and_trash_are_rejected_across_connections() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir = std::env::temp_dir().join(format!("paperroom-properties-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir)?;
        let path = dir.join("notes.sqlite3");
        let mut first = Notebook::open(&path, false)?;
        let second = Notebook::open(&path, false)?;
        let id = first.add(None, "共享", "")?;
        first.compare_set_property(id, "狀態", None, Some(""))?;
        assert!(second.compare_set_property(id, "狀態", None, Some("遺失")).is_err());
        second.compare_set_property(id, "狀態", Some(""), Some("更新"))?;
        assert!(first.compare_set_property(id, "狀態", Some(""), None).is_err());
        assert_eq!(first.properties(id)?[0].1, "更新");
        first.trash(id)?;
        assert!(second.compare_set_property(id, "狀態", Some("更新"), None).is_err());
        first.restore(id)?;
        assert_eq!(first.properties(id)?[0].1, "更新");
        drop(second); drop(first);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

}
