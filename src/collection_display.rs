//! Collection-specific columns and read-only file values from imported Bases views.
use super::{Notebook, Result, properties::key_checked};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::HashSet;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS collection_display_columns (
    collection_id INTEGER PRIMARY KEY REFERENCES collections(id) ON DELETE CASCADE,
    columns_json TEXT NOT NULL
);";

pub(super) const OBSIDIAN_VALUES_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS obsidian_base_file_columns (
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    PRIMARY KEY(collection_id, key)
);
CREATE TABLE IF NOT EXISTS obsidian_base_file_values (
    collection_id INTEGER NOT NULL,
    note_id INTEGER NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY(collection_id, note_id, key),
    FOREIGN KEY(collection_id, note_id)
        REFERENCES collection_notes(collection_id, note_id) ON DELETE CASCADE,
    FOREIGN KEY(collection_id, key)
        REFERENCES obsidian_base_file_columns(collection_id, key) ON DELETE CASCADE
);";

pub(crate) const MAX_DISPLAY_COLUMNS: usize = 4;
pub(crate) const DISPLAY_KEY_RESULTS: usize = 60;

fn validate_keys(keys: &[String]) -> Result<()> {
    if keys.len() > MAX_DISPLAY_COLUMNS {
        return Err(format!("最多顯示 {MAX_DISPLAY_COLUMNS} 個額外欄位。").into());
    }
    let mut seen = HashSet::new();
    for key in keys {
        if key_checked(key)? != key || !seen.insert(key) {
            return Err("欄位名稱必須與既有名稱相同，且每個欄位只能出現一次。".into());
        }
    }
    Ok(())
}

fn read_columns(db: &Connection, collection: i64) -> Result<Option<Vec<String>>> {
    let raw: Option<String> = db.query_row(
        "SELECT d.columns_json FROM collections c
         LEFT JOIN collection_display_columns d ON d.collection_id=c.id WHERE c.id=?1",
        [collection], |row| row.get(0),
    ).optional()?.ok_or("找不到這個集合。")?;
    raw.map(|raw| -> Result<_> {
        let keys: Vec<String> = serde_json::from_str(&raw)?;
        validate_keys(&keys)?;
        Ok(keys)
    }).transpose()
}

impl Notebook {
    /// None uses the original display. Some(empty) shows only the built-in columns.
    pub(crate) fn collection_display_columns(&self, collection: i64) -> Result<Option<Vec<String>>> {
        read_columns(&self.connection, collection)
    }

    pub(crate) fn collection_display_keys(&self, collection: i64, query: &str) -> Result<(Vec<String>, bool)> {
        let mut statement = self.connection.prepare(
            "SELECT key FROM property_definitions WHERE instr(key,?2)>0 OR ?2=''
             UNION
             SELECT p.key FROM collection_notes c JOIN notes n ON n.id=c.note_id
             JOIN note_properties p ON p.note_id=n.id
             WHERE c.collection_id=?1 AND n.trash_root IS NULL AND (instr(p.key,?2)>0 OR ?2='')
             UNION
             SELECT f.key FROM obsidian_base_file_columns f
             WHERE f.collection_id=?1 AND (instr(f.key,?2)>0 OR ?2='')
             ORDER BY key LIMIT ?3",
        )?;
        let mut keys = statement.query_map(params![collection, query, DISPLAY_KEY_RESULTS as i64 + 1], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        let more = keys.len() > DISPLAY_KEY_RESULTS;
        keys.truncate(DISPLAY_KEY_RESULTS);
        Ok((keys, more))
    }

    /// Use imported display-only snapshots in their original collection.
    #[cfg(any(windows, test))]
    pub(crate) fn collection_display_values(
        &self, collection: i64, key: &str,
    ) -> Result<std::collections::HashMap<i64, String>> {
        let key = key_checked(key)?;
        let mut statement = self.connection.prepare(
            "SELECT c.note_id, CASE WHEN fc.key IS NULL THEN p.value ELSE f.value END
             FROM collection_notes c
             JOIN notes n ON n.id=c.note_id
             LEFT JOIN note_properties p ON p.note_id=c.note_id AND p.key=?2
             LEFT JOIN obsidian_base_file_columns fc ON fc.collection_id=c.collection_id AND fc.key=?2
             LEFT JOIN obsidian_base_file_values f ON f.collection_id=c.collection_id
                 AND f.note_id=c.note_id AND f.key=?2
             WHERE c.collection_id=?1 AND n.trash_root IS NULL
                 AND ((fc.key IS NULL AND p.value IS NOT NULL) OR f.value IS NOT NULL)",
        )?;
        Ok(statement.query_map(params![collection,key], |row| Ok((row.get(0)?,row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn set_collection_display_columns(
        &self, collection: i64, expected: Option<&[String]>, next: Option<&[String]>,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if read_columns(&tx, collection)?.as_deref() != expected {
            return Err("其他視窗已更新這個集合的欄位顯示。請重新開啟設定。".into());
        }
        if expected == next { return Ok(()); }
        if let Some(keys) = next {
            validate_keys(keys)?;
            for key in keys.iter().filter(|key| !expected.is_some_and(|old| old.contains(key))) {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM property_definitions WHERE key=?2)
                       OR EXISTS(SELECT 1 FROM collection_notes c JOIN notes n ON n.id=c.note_id
                           JOIN note_properties p ON p.note_id=n.id
                           WHERE c.collection_id=?1 AND n.trash_root IS NULL AND p.key=?2)
                       OR EXISTS(SELECT 1 FROM obsidian_base_file_columns f
                           WHERE f.collection_id=?1 AND f.key=?2)",
                    params![collection, key], |row| row.get(0),
                )?;
                if !exists { return Err(format!("找不到可顯示的欄位「{key}」。").into()); }
            }
            tx.execute(
                "INSERT INTO collection_display_columns(collection_id,columns_json) VALUES(?1,?2)
                 ON CONFLICT(collection_id) DO UPDATE SET columns_json=excluded.columns_json",
                params![collection, serde_json::to_string(keys)?],
            )?;
        } else {
            tx.execute("DELETE FROM collection_display_columns WHERE collection_id=?1", [collection])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "collection_display_tests.rs"]
mod tests;
