//! Membership and ordering stay in collection views. These preferences only hide rendered groups.
use super::{Notebook, Result, collection_views::{CollectionLayout, CollectionView, collection_view_rows}, typed_properties};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use std::{borrow::Cow, collections::HashSet};

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS collection_group_folds (
    view_id INTEGER NOT NULL REFERENCES collection_views(id) ON DELETE CASCADE,
    grouping_key TEXT NOT NULL,
    grouping_kind TEXT NOT NULL,
    group_json TEXT NOT NULL,
    PRIMARY KEY(view_id, grouping_key, grouping_kind, group_json)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS collection_group_folds_by_key ON collection_group_folds(grouping_key, grouping_kind);
CREATE TRIGGER IF NOT EXISTS reset_collection_group_folds AFTER UPDATE OF group_key ON collection_views
WHEN OLD.group_key IS NOT NEW.group_key
BEGIN
    DELETE FROM collection_group_folds WHERE view_id=NEW.id;
END;";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GroupValue(pub(crate) Option<String>);

impl GroupValue {
    pub(crate) fn label(&self) -> Cow<'_, str> {
        match self.0.as_deref() {
            None => "未設定".into(),
            Some(value) if value.contains(['\r', '\n', '\t']) => value.replace(['\r', '\n', '\t'], " ").into(),
            Some(value) => value.into(),
        }
    }

    pub(crate) fn fold_label(&self) -> String {
        match self.0.as_deref() {
            None => "未設定（空白）".into(),
            Some(value) => {
                let text = value.chars().take(72).collect::<String>()
                    .replace('\\', "\\\\").replace('\r', "\\r").replace('\n', "\\n").replace('\t', "\\t");
                let suffix = if value.chars().nth(72).is_some() { "…" } else { "" };
                format!("「{text}{suffix}」")
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct GroupAxis {
    key: String,
    kind: &'static str,
}

impl GroupAxis {
    fn for_view(db: &Connection, view: &CollectionView) -> Result<Option<Self>> {
        if view.presentation.calendar_key.is_some() || view.presentation.timeline_mode {
            return Ok(None);
        }
        if let Some(key) = &view.presentation.group_key {
            let kind = typed_properties::lookup(db, key)?.map_or("text", |kind| kind.code());
            return Ok(Some(Self { key: key.clone(), kind }));
        }
        Ok((view.layout == CollectionLayout::Board).then(|| Self { key: String::new(), kind: "status" }))
    }
}

pub(crate) struct GroupFolds {
    axis: GroupAxis,
    folded: HashSet<GroupValue>,
}

impl GroupFolds {
    pub(crate) fn contains(&self, group: &GroupValue) -> bool { self.folded.contains(group) }
}

impl Notebook {
    pub(crate) fn collection_group_folds(&self, view: &CollectionView) -> Result<Option<GroupFolds>> {
        let Some(axis) = GroupAxis::for_view(&self.connection, view)? else { return Ok(None); };
        let mut stmt = self.connection.prepare(
            "SELECT group_json FROM collection_group_folds WHERE view_id=?1 AND grouping_key=?2 AND grouping_kind=?3",
        )?;
        let rows = stmt.query_map(params![view.id, axis.key, axis.kind], |row| row.get::<_, String>(0))?;
        let folded = rows.map(|row| Ok(GroupValue(serde_json::from_str(&row?)?)))
            .collect::<Result<_>>()?;
        Ok(Some(GroupFolds { axis, folded }))
    }

    pub(crate) fn set_collection_group_fold(
        &self, collection: i64, view: &CollectionView, snapshot: &GroupFolds,
        change: Option<(&GroupValue, bool)>,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if collection_view_rows(&tx, collection, Some(view.id))?.first() != Some(view)
            || GroupAxis::for_view(&tx, view)?.as_ref() != Some(&snapshot.axis)
        {
            return Err("分組或檢視已更新。請重新開啟後再操作。".into());
        }
        let axis = &snapshot.axis;
        if let Some((group, fold)) = change {
            let raw = serde_json::to_string(&group.0)?;
            if fold {
                let count: i64 = tx.query_row(
                    "SELECT count(*) FROM collection_group_folds WHERE view_id=?1", [view.id], |row| row.get(0),
                )?;
                if count >= super::collections::MAX_ITEMS {
                    return Err("已達收合記錄上限。請先展開一些或選擇「展開所有群組」。".into());
                }
                tx.execute(
                    "INSERT OR IGNORE INTO collection_group_folds(view_id,grouping_key,grouping_kind,group_json)
                     VALUES(?1,?2,?3,?4)", params![view.id, axis.key, axis.kind, raw],
                )?;
            } else {
                tx.execute(
                    "DELETE FROM collection_group_folds WHERE view_id=?1 AND grouping_key=?2 AND grouping_kind=?3 AND group_json=?4",
                    params![view.id, axis.key, axis.kind, raw],
                )?;
            }
        } else {
            tx.execute("DELETE FROM collection_group_folds WHERE view_id=?1", [view.id])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "collection_group_folds_tests.rs"]
mod tests;
