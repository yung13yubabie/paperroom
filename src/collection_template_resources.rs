//! References the exact objects that were explicitly chosen for template reuse.
use super::super::super::{Notebook, Result, attachments};
use super::super::{MAX_RELATIONS, checked_label, write_relation};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashSet;
use std::path::Path;

pub(crate) const TEMPLATE_RESOURCE_SCHEMA: &str = "
CREATE TABLE collection_template_resources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id INTEGER NOT NULL REFERENCES collection_templates(id) ON DELETE CASCADE,
    by_note INTEGER REFERENCES notes(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK(kind IN ('relation','attachment')),
    target_note INTEGER REFERENCES notes(id) ON DELETE SET NULL,
    attachment_id INTEGER REFERENCES attachments(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    detail TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    CHECK((kind='relation' AND bytes=0 AND attachment_id IS NULL)
       OR (kind='attachment' AND bytes BETWEEN 1 AND 8388608 AND target_note IS NULL))
);
CREATE INDEX collection_template_resources_template ON collection_template_resources(template_id,id);
CREATE INDEX collection_template_resources_attachment ON collection_template_resources(attachment_id);
CREATE INDEX collection_template_resources_source ON collection_template_resources(by_note);
CREATE INDEX collection_template_resources_target ON collection_template_resources(target_note);";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TemplateResource {
    id: i64,
    source: Option<i64>,
    name: String,
    detail: String,
    kind: ResourceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResourceKind {
    Relation(Option<i64>),
    Attachment { id: Option<i64>, bytes: i64 },
}

impl TemplateResource {
    pub fn label(&self) -> String {
        match &self.kind {
            ResourceKind::Relation(Some(target)) => format!("關聯　{} → {}  (#{target})", self.name, self.detail),
            ResourceKind::Relation(None) => format!("關聯　{} → {}（找不到原目標）", self.name, self.detail),
            ResourceKind::Attachment { id: Some(id), bytes } =>
                format!("附件　{}  (#{id}，{bytes} B)", self.name),
            ResourceKind::Attachment { id: None, .. } => format!("附件　{}（找不到原檔）", self.name),
        }
    }
}

pub(super) fn options(db: &Notebook, note: i64) -> Result<Vec<TemplateResource>> {
    let mut choices = db.relations(note)?.into_iter().map(|(name, target)| TemplateResource {
        id: 0, source: Some(note), name, detail: target.title, kind: ResourceKind::Relation(Some(target.id)),
    }).collect::<Vec<_>>();
    choices.extend(db.list_attachments(note)?.into_iter().map(|attachment| TemplateResource {
        id: 0, source: Some(note), name: attachment.name, detail: attachment.kind,
        kind: ResourceKind::Attachment { id: Some(attachment.id), bytes: attachment.size },
    }));
    Ok(choices)
}

pub(super) fn read(db: &Connection, template_id: i64) -> Result<Vec<TemplateResource>> {
    let mut stmt = db.prepare(
        "SELECT r.id,r.by_note,r.kind,r.target_note,r.attachment_id,r.name,coalesce(n.title,r.detail),r.bytes
         FROM collection_template_resources r LEFT JOIN notes n ON n.id=r.target_note
         WHERE r.template_id=?1 ORDER BY r.id LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![template_id, MAX_RELATIONS + attachments::MAX_PER_NOTE + 1], |row| {
        let kind = match row.get_ref(2)?.as_str()? {
            "relation" => ResourceKind::Relation(row.get(3)?),
            "attachment" => ResourceKind::Attachment { id: row.get(4)?, bytes: row.get(7)? },
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        Ok(TemplateResource { id: row.get(0)?, source: row.get(1)?, name: row.get(5)?, detail: row.get(6)?, kind })
    })?.collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() as i64 > MAX_RELATIONS + attachments::MAX_PER_NOTE { return Err("範本的關聯或附件超過上限。".into()); }
    Ok(rows)
}

fn checked_attachment(resource: &TemplateResource, bytes: i64) -> Result<()> {
    if resource.name.chars().count() > 255 || resource.name.chars().any(char::is_control)
        || resource.name.contains(['/', '\\', ':'])
        || attachments::file_kind(Path::new(&resource.name))? != resource.detail
        || !(1..=attachments::MAX_FILE_BYTES as i64).contains(&bytes)
    {
        return Err(format!("附件「{}」的檔名、格式或大小不符。", resource.name).into());
    }
    Ok(())
}

fn verify(
    db: &Connection, collection: i64, references: &[TemplateResource],
) -> Result<(i64, i64)> {
    let mut sources = HashSet::new();
    let mut relations = HashSet::new();
    let mut files = HashSet::new();
    let mut total = 0;
    for resource in references {
        let obsolete = || format!("「{}」已無法使用。可先從範本移除這個選擇，或只套用正文和型別欄位。", resource.label());
        let source = resource.source.ok_or_else(obsolete)?;
        if sources.insert(source) {
            let member = db.prepare_cached(
                "SELECT 1 FROM collection_notes c JOIN notes n ON n.id=c.note_id
                 WHERE c.collection_id=?1 AND c.note_id=?2 AND n.trash_root IS NULL",
            )?.exists(params![collection, source])?;
            if !member { return Err(obsolete().into()); }
        }
        match resource.kind {
            ResourceKind::Relation(target) => {
                let name = checked_label(&resource.name)?;
                let target = target.ok_or_else(obsolete)?;
                if !relations.insert((name, target)) { return Err("範本重複選取相同的關聯。".into()); }
                let exists = db.prepare_cached(
                    "SELECT 1 FROM note_relations r JOIN notes n ON n.id=r.target_id
                     WHERE r.source_id=?1 AND r.name=?2 AND r.target_id=?3 AND n.trash_root IS NULL",
                )?.exists(params![source,name,target])?;
                if !exists { return Err(obsolete().into()); }
            }
            ResourceKind::Attachment { id, bytes } => {
                let id = id.ok_or_else(obsolete)?;
                checked_attachment(resource, bytes)?;
                if !files.insert(id) { return Err("範本重複選取相同的附件。".into()); }
                let exists = db.prepare_cached(
                    "SELECT 1 FROM attachments WHERE note_id=?1 AND id=?2 AND name=?3 AND kind=?4 AND length(content)=?5",
                )?.exists(params![source,id,resource.name,resource.detail,bytes])?;
                if !exists { return Err(obsolete().into()); }
                total += bytes;
            }
        }
    }
    if relations.len() as i64 > MAX_RELATIONS || files.len() as i64 > attachments::MAX_PER_NOTE {
        return Err("範本的關聯或附件超過每篇筆記上限。".into());
    }
    Ok((files.len() as i64, total))
}

pub(super) fn capture(tx: &Transaction<'_>, collection: i64, template: i64, references: &[TemplateResource]) -> Result<()> {
    verify(tx, collection, references)?;
    for resource in references {
        let (kind, target, attachment, bytes) = match resource.kind {
            ResourceKind::Relation(target) => ("relation", target, None, 0),
            ResourceKind::Attachment { id, bytes } => ("attachment", None, id, bytes),
        };
        tx.execute(
            "INSERT INTO collection_template_resources(template_id,by_note,kind,target_note,attachment_id,name,detail,bytes)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![template, resource.source, kind, target, attachment, resource.name, resource.detail, bytes],
        )?;
    }
    Ok(())
}

pub(super) fn retain(tx: &Transaction<'_>, template: i64, draft: &[TemplateResource]) -> Result<()> {
    let original = read(tx, template)?;
    let valid: HashSet<_> = original.iter().map(|item| item.id).collect();
    let keep: HashSet<_> = draft.iter().map(|item| item.id).collect();
    if keep.len() != draft.len() || !keep.is_subset(&valid) { return Err("範本項目已變更，請重新檢閱。".into()); }
    for item in original {
        if !keep.contains(&item.id) {
            tx.execute("DELETE FROM collection_template_resources WHERE template_id=?1 AND id=?2", params![template,item.id])?;
        }
    }
    Ok(())
}

pub(super) fn apply(tx: &Transaction<'_>, collection: i64, note: i64, references: &[TemplateResource]) -> Result<()> {
    let (files, bytes) = verify(tx, collection, references)?;
    if files > 0 {
        let stored: i64 = tx.query_row("SELECT coalesce(sum(length(content)),0) FROM attachments", [], |row| row.get(0))?;
        if stored > attachments::MAX_STORAGE_BYTES - bytes {
            return Err("附件複本會超過工作區 128 MiB 上限。可取消選取部分附件，或只套用正文和型別欄位。".into());
        }
    }
    for resource in references {
        match resource.kind {
            ResourceKind::Relation(Some(target)) => {
                write_relation(tx, note, &resource.name, target, true)?;
            }
            ResourceKind::Attachment { id: Some(attachment), .. } => {
                let data: Vec<u8> = tx.query_row("SELECT content FROM attachments WHERE id=?1", [attachment], |row| row.get(0))
                    .optional()?.ok_or("找不到來源附件。")?;
                if !attachments::has_expected_signature(&resource.detail, &data) {
                    return Err(format!("附件「{}」內容與格式不符。", resource.name).into());
                }
                tx.execute("INSERT INTO attachments(note_id,name,kind,content) VALUES (?1,?2,?3,?4)",
                    params![note,resource.name,resource.detail,data])?;
            }
            _ => return Err("範本關聯或附件已失效。".into()),
        }
    }
    Ok(())
}
