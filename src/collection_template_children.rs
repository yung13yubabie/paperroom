//! Explicit page snapshots with parent-first template-local hierarchy.
use super::{checked_title, validate_template_body};
use super::super::super::{Notebook, Result, MAX_NOTE_BYTES, insert};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashSet;

pub(crate) const TEMPLATE_CHILD_SCHEMA: &str = "
CREATE TABLE collection_template_children (
    template_id INTEGER NOT NULL REFERENCES collection_templates(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0 AND ordinal < 64),
    title TEXT NOT NULL,
    body TEXT NOT NULL CHECK(length(CAST(body AS BLOB))<=4194304),
    PRIMARY KEY(template_id,ordinal)
);";

pub(crate) const TEMPLATE_NESTED_SCHEMA: &str = "
ALTER TABLE collection_template_children
    ADD COLUMN parent_ordinal INTEGER CHECK(parent_ordinal IS NULL OR (parent_ordinal >= 0 AND parent_ordinal < ordinal));";

const MAX_CHILDREN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TemplateChild {
    pub title: String,
    pub body: String,
    pub parent: Option<usize>,
}

pub(crate) struct ChildDraft {
    pub id: i64,
    version: i64,
    pub content: TemplateChild,
}

pub(super) fn paths(children: &[TemplateChild]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::with_capacity(children.len());
    for child in children {
        let path = child.parent.and_then(|i| paths.get(i))
            .map_or_else(|| child.title.clone(), |parent| format!("{parent} / {}", child.title));
        paths.push(path);
    }
    paths
}

fn remaining_ordinals(children: &[TemplateChild], removed: usize) -> Vec<Option<usize>> {
    let mut map = Vec::<Option<usize>>::with_capacity(children.len());
    let mut next = 0;
    for (i, child) in children.iter().enumerate() {
        if i == removed || child.parent.is_some_and(|parent| map.get(parent).copied().flatten().is_none()) {
            map.push(None);
        } else {
            map.push(Some(next));
            next += 1;
        }
    }
    map
}

pub(super) fn removal_count(children: &[TemplateChild], page: usize) -> usize {
    remaining_ordinals(children, page).iter().filter(|next| next.is_none()).count()
}

pub(super) fn remove(children: &mut Vec<TemplateChild>, page: usize) {
    let map = remaining_ordinals(children, page);
    let old = std::mem::take(children);
    for (idx, mut child) in old.into_iter().enumerate() {
        if map[idx].is_some() {
            child.parent = child.parent.and_then(|parent| map[parent]);
            children.push(child);
        }
    }
}

pub(super) fn options(db: &Connection, note: i64) -> Result<Vec<(i64, String)>> {
    let mut query = db.prepare("SELECT id,title FROM notes WHERE parent=?1 AND trash_root IS NULL ORDER BY id")?;
    Ok(query.query_map([note], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
}

pub(super) fn source(db: &Notebook, parent: i64, id: i64) -> Result<ChildDraft> {
    db.connection.query_row(
        "SELECT title,body,version FROM notes WHERE id=?1 AND parent=?2 AND trash_root IS NULL",
        params![id,parent], |row| Ok(ChildDraft {
            id, version: row.get(2)?, content: TemplateChild { title: row.get(0)?, body: row.get(1)?, parent: None },
        }),
    ).optional()?.ok_or_else(|| "這個子頁已移動、移除或在垃圾桶，請重新開啟。".into())
}

pub(super) fn capture(
    tx: &Transaction<'_>, parent: i64, template: i64, root_size: usize, drafts: &[ChildDraft],
) -> Result<()> {
    let mut ids = HashSet::new();
    let mut check = tx.prepare_cached(
        "SELECT 1 FROM notes WHERE id=?1 AND parent=?2 AND version=?3 AND trash_root IS NULL",
    )?;
    for draft in drafts {
        if !ids.insert(draft.id) { return Err("範本重複選取了同一個子頁。".into()); }
        if !check.exists(params![draft.id,parent,draft.version])? {
            return Err("選取的子頁已更改、移動或移到垃圾桶，請重新檢閱。".into());
        }
    }
    let selected = drafts.iter().map(|draft| TemplateChild { parent: None, ..draft.content.clone() }).collect::<Vec<_>>();
    replace(tx, template, root_size, &selected)
}

fn validate(root_size: usize, children: &[TemplateChild]) -> Result<()> {
    if children.len() > MAX_CHILDREN || children.iter().fold(root_size, |size, child| size.saturating_add(child.body.len())) > MAX_NOTE_BYTES {
        return Err("每份範本最多 64 個子頁，正文和子頁正文合計最多 4 MiB。".into());
    }
    for (ordinal, child) in children.iter().enumerate() {
        if child.parent.is_some_and(|parent| parent >= ordinal) { return Err("範本子頁的上層順序有誤。".into()); }
        checked_title(&child.title)?;
        validate_template_body(&child.body)?;
    }
    Ok(())
}

pub(super) fn read(db: &Connection, template: i64) -> Result<Vec<TemplateChild>> {
    let mut query = db.prepare(
        "SELECT ordinal,title,body,parent_ordinal FROM collection_template_children WHERE template_id=?1 ORDER BY ordinal",
    )?;
    let mut rows = query.query([template])?;
    let mut children = Vec::new();
    while let Some(row) = rows.next()? {
        let ordinal = usize::try_from(row.get::<_, i64>(0)?)?;
        let parent = row.get::<_, Option<i64>>(3)?.map(usize::try_from).transpose()?;
        if ordinal != children.len() || parent.is_some_and(|parent| parent >= ordinal) {
            return Err("範本子頁的上層順序有誤。".into());
        }
        children.push(TemplateChild { title: row.get(1)?, body: row.get(2)?, parent });
    }
    Ok(children)
}

pub(super) fn replace(
    tx: &Transaction<'_>, template: i64, root_size: usize, children: &[TemplateChild],
) -> Result<()> {
    validate(root_size, children)?;
    tx.execute("DELETE FROM collection_template_children WHERE template_id=?1", [template])?;
    let mut put = tx.prepare_cached(
        "INSERT INTO collection_template_children(template_id,ordinal,title,body,parent_ordinal) VALUES(?1,?2,?3,?4,?5)",
    )?;
    for (ordinal, child) in children.iter().enumerate() {
        put.execute(params![template, ordinal as i64, checked_title(&child.title)?, child.body, child.parent.map(|parent| parent as i64)])?;
    }
    Ok(())
}

pub(super) fn apply(
    tx: &Transaction<'_>, parent: i64, root_size: usize, children: &[TemplateChild],
) -> Result<()> {
    validate(root_size, children)?;
    let mut ids = Vec::with_capacity(children.len());
    for child in children {
        let parent = child.parent.map_or(parent, |ordinal| ids[ordinal]);
        ids.push(insert(tx, Some(parent), &child.title, &child.body)?);
    }
    Ok(())
}
