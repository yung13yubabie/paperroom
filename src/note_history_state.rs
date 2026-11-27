//! Metadata stored with new revisions. Old text-only rows have no state row.
use super::{Result, attachments, collection_views, properties, typed_properties};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const SCHEMA: &str = include_str!("note_history_state.sql");

type Properties = BTreeMap<String, String>;

pub(super) fn checkpoint(tx: &Transaction<'_>, note: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO capture_note_history(note_id,version,title,body)
         SELECT id,version,title,body FROM notes WHERE id=?1 AND trash_root IS NULL",
        [note],
    )?;
    Ok(())
}

pub(super) fn snapshot_properties(db: &Connection, revision: i64) -> Result<Option<Properties>> {
    let raw: Option<String> = db.query_row(
        "SELECT properties_json FROM note_revision_state WHERE revision_id=?1", [revision],
        |row| row.get(0),
    ).optional()?;
    Ok(raw.map(|json| serde_json::from_str(&json)).transpose()?)
}

pub(super) struct AttachmentSummary {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) bytes: i64,
}

pub(super) fn summaries(db: &Connection, revision: i64) -> Result<Vec<AttachmentSummary>> {
    let mut query = db.prepare(
        "SELECT original_id,name,length(content) FROM note_revision_attachments WHERE revision_id=?1 ORDER BY original_id",
    )?;
    Ok(query.query_map([revision], |row| Ok(AttachmentSummary {
        id: row.get(0)?, name: row.get(1)?, bytes: row.get(2)?,
    }))?.collect::<rusqlite::Result<_>>()?)
}

// Identity and all saved data must match. In particular, an unchanged name
// and size are insufficient for an attachment refreshed from a local Vault.
fn unchanged_ids(db: &Connection, note: i64, revision: i64) -> Result<BTreeSet<i64>> {
    let mut query = db.prepare(
        "SELECT a.id FROM attachments a
         LEFT JOIN attachment_origins o ON o.attachment_id=a.id
         JOIN note_revision_attachments h ON h.original_id=a.id
         WHERE a.note_id=?1 AND h.revision_id=?2 AND a.name=h.name AND a.kind=h.kind
             AND a.content=h.content AND o.relative_path IS h.origin",
    )?;
    Ok(query.query_map(params![note, revision], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?)
}

// Only an existing protected source that the snapshot would remove or alter
// blocks restoration. Unrelated files on the same receipt remain editable.
pub(super) fn changes_base_source(db: &Connection, note: i64, revision: i64) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(
        SELECT 1 FROM base_card_receipts b JOIN attachments a ON a.id=b.attachment
        LEFT JOIN attachment_origins o ON o.attachment_id=a.id
        WHERE a.note_id=?1 AND NOT EXISTS(
            SELECT 1 FROM note_revision_attachments h WHERE h.revision_id=?2 AND h.original_id=a.id
                AND h.name=a.name AND h.kind=a.kind AND h.content=a.content AND h.origin IS o.relative_path
        ))", params![note,revision], |r|r.get(0))?)
}

pub(super) fn current_properties(db: &Connection, note: i64) -> Result<Properties> {
    let mut query = db.prepare("SELECT key,value FROM note_properties WHERE note_id=?1")?;
    Ok(query.query_map([note], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
}

pub(super) fn attachments_changed(db: &Connection, note: i64, revision: i64) -> Result<bool> {
    let same = unchanged_ids(db, note, revision)?.len() as i64;
    let (live, saved): (i64, i64) = db.query_row(
        "SELECT (SELECT count(*) FROM attachments WHERE note_id=?1),
                (SELECT count(*) FROM note_revision_attachments WHERE revision_id=?2)",
        params![note, revision], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(same != live || same != saved)
}

pub(super) fn apply_properties(
    tx: &Transaction<'_>, note: i64, current: &Properties, desired: &Properties,
) -> Result<()> {
    if desired.len() > properties::MAX_PROPERTIES as usize {
        return Err("歷史屬性已超過每篇筆記的容量上限。".into());
    }
    let removed = current.keys().filter(|key| !desired.contains_key(*key));
    let updated = desired.iter().filter(|(key, value)| current.get(*key) != Some(*value));
    let mut dates = Vec::new();
    for key in removed {
        properties::delete_value(tx, note, key)?;
        if typed_properties::lookup(tx, key)? == Some(typed_properties::PropertyType::Date) {
            dates.push((key.as_str(), None));
        }
    }
    for (key, value) in updated {
        if properties::key_checked(key)? != key.as_str() || value.len() > properties::MAX_VALUE_BYTES || value.contains('\0') {
            return Err("歷史屬性與目前支援的格式不符。".into());
        }
        typed_properties::validate_write(tx, key, value)?;
        properties::write_value(tx, note, key, value, !current.contains_key(key))?;
        if typed_properties::lookup(tx, key)? == Some(typed_properties::PropertyType::Date) {
            dates.push((key.as_str(), collection_views::CalendarDay::parse(value)));
        }
    }
    // Validate the final set so paired date edits can be restored together.
    for date in dates {
        collection_views::validate_collection_field_dates(tx, note, [date, ("", None)])?;
    }
    Ok(())
}

struct StoredAttachment {
    id: i64,
    name: String,
    kind: String,
    content: Vec<u8>,
    origin: Option<String>,
}

/// Only changed or removed live attachments can invalidate those references.
pub(super) fn protected_attachment_changes(db: &Connection, note: i64, revision: i64) -> Result<Vec<String>> {
    let same = unchanged_ids(db,note,revision)?;
    protected_changes_excluding(db,note,&same)
}

fn protected_changes_excluding(db:&Connection,note:i64,same:&BTreeSet<i64>)->Result<Vec<String>> {
    let mut query = db.prepare("SELECT a.id,a.name,
        EXISTS(SELECT 1 FROM pdf_annotations WHERE attachment_id=a.id),
        EXISTS(SELECT 1 FROM pdf_excerpt_origins WHERE attachment_id=a.id),
        EXISTS(SELECT 1 FROM board_group_sources s JOIN pdf_annotations p ON s.annotation_id=p.id WHERE p.attachment_id=a.id),
        EXISTS(SELECT 1 FROM collection_template_resources WHERE attachment_id=a.id),
        EXISTS(SELECT 1 FROM board_image_cards WHERE attachment_id=a.id),
        EXISTS(SELECT 1 FROM zotero_imports WHERE original_attachment=a.id)
        FROM attachments a WHERE a.note_id=?1 ORDER BY a.id")?;
    let mut items=Vec::new();
    let mut rows=query.query([note])?;
    while let Some(row)=rows.next()? {
        let id:i64=row.get(0)?;
        if same.contains(&id) { continue; }
        let mut uses=Vec::new();
        for (i,kind) in ["PDF 標註","PDF 摘錄卡","白板分區釘選","資料列範本","白板圖片卡","Zotero 書目"].iter().enumerate() {
            if row.get::<_,bool>(i+2)? { uses.push(*kind); }
        }
        if !uses.is_empty() {
            let name:String=row.get(1)?;
            items.push(format!("{}　[#{id}]（{}）",super::note_history::preview_field(&name),uses.join("、")));
        }
    }
    Ok(items)
}

pub(super) struct AttachmentRestore {
    remove: Vec<i64>,
    replace: Vec<StoredAttachment>,
}

impl AttachmentRestore {
    pub(super) fn read(tx: &Transaction<'_>, note: i64, revision: i64) -> Result<Self> {
        if changes_base_source(tx,note,revision)? {
            return Err(super::note_history::BASE_SOURCE_GUIDANCE.into());
        }
        let same = unchanged_ids(tx, note, revision)?;
        let blocked=protected_changes_excluding(tx,note,&same)?;
        if !blocked.is_empty() {
            return Err(format!("附件仍被其他資料使用，無法整份還原：{}",blocked.join("；")).into());
        }
        let (count, total, elsewhere): (i64, i64, i64) = tx.query_row(
            "SELECT count(*),coalesce(sum(length(content)),0),
              (SELECT coalesce(sum(length(content)),0) FROM attachments WHERE note_id<>?1)
             FROM note_revision_attachments WHERE revision_id=?2",
            params![note, revision], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if count > attachments::MAX_PER_NOTE || total > attachments::MAX_STORAGE_BYTES - elsewhere {
            return Err("還原會超過筆記或工作區的附件容量。".into());
        }

        let mut live = tx.prepare("SELECT id,name FROM attachments WHERE note_id=?1 ORDER BY id")?;
        let live = live.query_map([note], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
        let mut history = tx.prepare(
            "SELECT original_id,name,kind,content,origin FROM note_revision_attachments
             WHERE revision_id=?1 ORDER BY original_id",
        )?;
        let mut rows = history.query([revision])?;
        let mut replacements = Vec::new();
        let mut retained = BTreeSet::new();
        while let Some(row) = rows.next()? {
            let id = row.get(0)?;
            retained.insert(id);
            if same.contains(&id) { continue; }
            let item = StoredAttachment {
                id, name: row.get(1)?, kind: row.get(2)?, content: row.get(3)?, origin: row.get(4)?,
            };
            if !attachments::has_expected_signature(&item.kind, &item.content) {
                return Err("歷史附件的原始內容與類型不符。".into());
            }
            replacements.push(item);
        }
        let remove = live.keys().filter(|id| !retained.contains(*id)).copied().collect::<Vec<_>>();
        Ok(Self { remove, replace: replacements })
    }

    pub(super) fn apply(self, tx: &Transaction<'_>, note: i64) -> Result<()> {
        for id in self.remove {
            tx.execute("DELETE FROM attachments WHERE id=?1 AND note_id=?2", params![id,note])?;
        }
        for item in self.replace {
            let changed = tx.execute(
                "INSERT INTO attachments(id,note_id,name,kind,content) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name,kind=excluded.kind,content=excluded.content
                 WHERE attachments.note_id=excluded.note_id",
                params![item.id,note,item.name,item.kind,item.content],
            )?;
            if changed != 1 { return Err("歷史附件識別已被另一篇筆記使用，已取消整份還原。".into()); }
            tx.execute("DELETE FROM attachment_origins WHERE attachment_id=?1", [item.id])?;
            if let Some(origin) = item.origin {
                tx.execute(
                    "INSERT INTO attachment_origins(attachment_id,relative_path) VALUES(?1,?2)",
                    params![item.id,origin],
                )?;
            }
        }
        Ok(())
    }
}
