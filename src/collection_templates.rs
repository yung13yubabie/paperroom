//! Collection-scoped recipes for independent note creation.
use super::{MAX_ITEMS, RelationMember, checked_label, require_member};
use super::super::{Notebook, Result, validate_body, insert};
use super::super::properties::{key_checked, write_value};
use super::super::typed_properties::{PropertyType, lookup};
use pulldown_cmark::{Event, Options, Parser, Tag};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;
use std::collections::{HashMap, HashSet};

#[path = "collection_template_resources.rs"]
mod resources;
pub(crate) use resources::TEMPLATE_RESOURCE_SCHEMA;
use resources::TemplateResource;

#[path = "collection_template_children.rs"]
mod children;
pub(crate) use children::{ChildDraft, TemplateChild, TEMPLATE_CHILD_SCHEMA, TEMPLATE_NESTED_SCHEMA};

pub(crate) const TEMPLATE_SCHEMA: &str = "
CREATE TABLE collection_templates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    name TEXT NOT NULL COLLATE NOCASE,
    default_title TEXT NOT NULL,
    status TEXT NOT NULL,
    body TEXT NOT NULL,
    fields_json TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK(revision >= 0),
    UNIQUE(collection_id,name)
);";

const MAX_TEMPLATES: i64 = 64;
const MAX_FIELDS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemplateField {
    pub key: String,
    pub kind: PropertyType,
    pub value: String,
}

#[derive(Default)]
pub(crate) struct TemplateApply {
    pub resources: bool,
    pub children: bool,
}

#[derive(Default)]
pub(crate) struct TemplateSelection<'a> {
    pub fields: &'a [usize],
    pub resources: &'a [usize],
    pub children: &'a [ChildDraft],
}

pub(crate) struct TemplateSource {
    note: i64,
    version: i64,
    pub title: String,
    pub status: String,
    pub body: String,
    pub fields: Vec<TemplateField>,
    pub resources: Vec<TemplateResource>,
    pub children: Vec<(i64, String)>,
}

#[derive(Clone)]
pub(crate) struct CollectionTemplate {
    pub id: i64,
    pub name: String,
    pub default_title: String,
    pub status: String,
    pub body: String,
    pub fields: Vec<TemplateField>,
    pub resources: Vec<TemplateResource>,
    pub children: Vec<TemplateChild>,
    revision: i64,
}

impl CollectionTemplate {
    pub(crate) fn blank(name: String, title: String, status: String, body: String) -> Self {
        Self { id: 0, name, default_title: title, status, body,
            fields: Vec::new(), resources: Vec::new(), children: Vec::new(), revision: 0 }
    }
    pub(crate) fn child_paths(&self) -> Vec<String> { children::paths(&self.children) }

    pub(crate) fn child_removal_count(&self, page: usize) -> usize { children::removal_count(&self.children, page) }

    pub(crate) fn remove_child(&mut self, page: usize) { children::remove(&mut self.children, page); }

}

pub(super) fn checked_title(value: &str) -> Result<&str> {
    let title = value.trim();
    if title.is_empty() || title.chars().any(char::is_control)
        || title.chars().count() > super::super::MAX_TITLE_CHARS
    {
        return Err("筆記標題需有內容、使用單行文字且最多 180 字元。".into());
    }
    Ok(title)
}

pub(crate) fn validate_template_body(body: &str) -> Result<()> {
    validate_body(body)?;
    let lower = body.to_ascii_lowercase();
    let raw_links = ["http:", "https:", "file:", "obsidian:", "paperroom:", "mailto:", "[["];
    let mut parser = Parser::new_ext(body, Options::ENABLE_WIKILINKS | Options::ENABLE_GFM);
    if raw_links.iter().any(|scheme| lower.contains(scheme))
        || parser.reference_definitions().iter().next().is_some()
        || parser.any(|event| matches!(event,
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) | Event::Html(_) | Event::InlineHtml(_)))
    {
        return Err("範本可保存 Markdown 內容。請先移除連結、附件引用、嵌入與 HTML。".into());
    }
    Ok(())
}

fn source_fields(db: &Connection, note: i64) -> Result<Vec<TemplateField>> {
    let mut stmt = db.prepare(
        "SELECT p.key,d.kind,p.value FROM note_properties p JOIN property_definitions d ON d.key=p.key
         WHERE p.note_id=?1 ORDER BY p.key LIMIT 129",
    )?;
    let fields = stmt.query_map([note], |row| Ok(TemplateField {
        key: row.get(0)?, kind: PropertyType::decode(row.get_ref(1)?.as_str()?)?, value: row.get(2)?,
    }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if fields.len() > MAX_FIELDS { return Err("來源筆記有超過上限的型別欄位。".into()); }
    Ok(fields.into_iter().filter(|field| field.value.len() <= super::super::MAX_VALUE_BYTES
        && field.kind.normalized(&field.value).is_some()).collect())
}

fn encode_fields(fields: &[TemplateField]) -> String {
    json!(fields.iter().map(|field| json!([field.key, field.kind.code(), field.value])).collect::<Vec<_>>()).to_string()
}

fn decode_fields(raw: &str) -> rusqlite::Result<Vec<TemplateField>> {
    let max = 7 * MAX_FIELDS * (super::super::MAX_KEY_BYTES + super::super::MAX_VALUE_BYTES + 32);
    if raw.len() > max { return Err(rusqlite::Error::InvalidQuery); }
    let fields: Vec<(String, String, String)> = serde_json::from_str(raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if fields.len() > MAX_FIELDS { return Err(rusqlite::Error::InvalidQuery); }
    fields.into_iter().map(|(key, kind, value)| Ok(TemplateField { key, kind: PropertyType::decode(&kind)?, value })).collect()
}

fn read_template(row: &rusqlite::Row<'_>) -> rusqlite::Result<CollectionTemplate> {
    Ok(CollectionTemplate { id: row.get(0)?, name: row.get(1)?, default_title: row.get(2)?, status: row.get(3)?,
        body: row.get(4)?, fields: decode_fields(row.get_ref(5)?.as_str()?)?, resources: Vec::new(), children: Vec::new(), revision: row.get(6)? })
}

fn ensure_current(tx: &Transaction<'_>, collection: i64, template: &CollectionTemplate) -> Result<()> {
    let found = tx.prepare("SELECT 1 FROM collection_templates WHERE collection_id=?1 AND id=?2 AND revision=?3")?
        .exists(params![collection, template.id, template.revision])?;
    if !found { return Err("範本已更改或移除，請重新開啟。".into()); }
    Ok(())
}

fn validate_fields(db: &Connection, collection: i64, fields: &[TemplateField]) -> Result<()> {
    if fields.len() > MAX_FIELDS { return Err("每份範本最多 128 個型別欄位。".into()); }
    let mut names = HashSet::new();
    let mut dates = HashMap::new();
    for field in fields {
        if key_checked(&field.key)? != field.key || !names.insert(field.key.as_str()) {
            return Err("範本欄位名稱無效或重複。".into());
        }
        if lookup(db, &field.key)? != Some(field.kind) {
            return Err(format!("「{}」的型別已變更。請修正或移除範本欄位。", field.key).into());
        }
        if field.value.len() > super::super::MAX_VALUE_BYTES || field.kind.normalized(&field.value).is_none() {
            return Err(format!("「{}」需要有效的{}。", field.key, field.kind.hint()).into());
        }
        if field.kind == PropertyType::Date { dates.insert(field.key.as_str(), field.value.trim()); }
    }
    if dates.is_empty() { return Ok(()); }
    let mut stmt = db.prepare(
        "SELECT DISTINCT calendar_key,calendar_end_key FROM collection_views
         WHERE collection_id=?1 AND calendar_end_key IS NOT NULL",
    )?;
    let mut pairs = stmt.query([collection])?;
    while let Some(row) = pairs.next()? {
        let start = row.get_ref(0)?.as_str()?;
        let end = row.get_ref(1)?.as_str()?;
        if let Some(end_value) = dates.get(end)
            && dates.get(start).is_none_or(|start_value| start_value > end_value)
        {
            return Err(format!("請在範本中同時選擇有效的「{start}」與「{end}」，起始日不能晚於結束日。").into());
        }
    }
    Ok(())
}

fn require_available_name(db: &Connection, collection: i64, name: &str, except: Option<i64>) -> Result<()> {
    let exists = db.prepare(
        "SELECT 1 FROM collection_templates WHERE collection_id=?1 AND name=?2
         AND (?3 IS NULL OR id<>?3)",
    )?.exists(params![collection, name, except])?;
    if exists { return Err("這個集合已有同名範本。".into()); }
    Ok(())
}

fn insert_recipe(
    tx: &Transaction<'_>, collection: i64, name: &str, title: &str, status: &str, body: &str,
    fields: &[TemplateField],
) -> Result<i64> {
    require_available_name(tx, collection, name, None)?;
    let changed = tx.execute(
        "INSERT INTO collection_templates(collection_id,name,default_title,status,body,fields_json)
         SELECT ?1,?2,?3,?4,?5,?6 WHERE (SELECT count(*) FROM collection_templates WHERE collection_id=?1)<?7",
        params![collection, name, title, status, body, encode_fields(fields), MAX_TEMPLATES],
    )?;
    if changed == 0 { return Err("每個集合最多可儲存 64 份範本。".into()); }
    Ok(tx.last_insert_rowid())
}

impl Notebook {
    pub(crate) fn collection_template_source(&self, collection: i64, member: RelationMember<'_>) -> Result<TemplateSource> {
        let snapshot = self.connection.is_autocommit().then(|| self.connection.unchecked_transaction()).transpose()?;
        require_member(&self.connection, collection, member)?;
        let note = self.note(member.note)?.ok_or("找不到來源筆記。")?;
        let fields = source_fields(&self.connection, member.note)?;
        let resources = resources::options(self, member.note)?;
        let children = children::options(&self.connection, member.note)?;
        if let Some(snapshot) = snapshot { snapshot.commit()?; }
        Ok(TemplateSource { note: member.note, version: note.version, title: note.row.title,
            status: member.status.to_owned(), body: note.body, fields, resources, children })
    }

    pub(crate) fn collection_template_child_source(&self, source: &TemplateSource, id: i64) -> Result<ChildDraft> {
        children::source(self, source.note, id)
    }

    pub(crate) fn capture_collection_template(
        &self, collection: i64, source: &TemplateSource, name: &str, body: &str, selected: TemplateSelection<'_>,
    ) -> Result<i64> {
        let name = checked_label(name)?;
        let title = checked_title(&source.title)?;
        let status = checked_label(&source.status)?;
        validate_template_body(body)?;
        let fields = selected.fields.iter().map(|index| source.fields.get(*index).cloned().ok_or("找不到選取的來源欄位。"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let refs = selected.resources.iter().map(|index| source.resources.get(*index).cloned().ok_or("找不到選取的關聯或附件。"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        require_member(&tx, collection, RelationMember { note: source.note, status })?;
        let actual_version: i64 = tx.query_row("SELECT version FROM notes WHERE id=?1", [source.note], |row| row.get(0))?;
        if actual_version != source.version { return Err("來源筆記已更改，請重新檢閱範本預覽。".into()); }
        validate_fields(&tx, collection, &fields)?;
        let actual_fields = source_fields(&tx, source.note)?;
        if !fields.iter().all(|field| actual_fields.contains(field)) {
            return Err("來源欄位已更改，請重新選取要存入範本的欄位。".into());
        }
        let id = insert_recipe(&tx, collection, name, title, status, body, &fields)?;
        resources::capture(&tx, collection, id, &refs)?;
        children::capture(&tx, source.note, id, body.len(), selected.children)?;
        tx.commit()?;
        Ok(id)
    }

    pub(crate) fn new_collection_template(&self, collection: i64, draft: &CollectionTemplate) -> Result<i64> {
        let name = checked_label(&draft.name)?;
        let title = checked_title(&draft.default_title)?;
        let status = checked_label(&draft.status)?;
        validate_template_body(&draft.body)?;
        if !draft.resources.is_empty() {
            return Err("直接建立的範本沒有可重用的來源關聯或附件。".into());
        }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if !tx.prepare("SELECT 1 FROM collections WHERE id=?1")?.exists([collection])? {
            return Err("集合已移除。".into());
        }
        validate_fields(&tx, collection, &draft.fields)?;
        let id = insert_recipe(&tx, collection, name, title, status, &draft.body, &draft.fields)?;
        children::replace(&tx, id, draft.body.len(), &draft.children)?;
        tx.commit()?;
        Ok(id)
    }

    pub(crate) fn collection_templates(&self, collection: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.connection.prepare(
            "SELECT id,name FROM collection_templates WHERE collection_id=?1 ORDER BY name COLLATE NOCASE,id",
        )?;
        Ok(stmt.query_map([collection], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn collection_template(&self, collection: i64, id: i64) -> Result<CollectionTemplate> {
        let snapshot = self.connection.is_autocommit().then(|| self.connection.unchecked_transaction()).transpose()?;
        let mut template = self.connection.query_row(
            "SELECT id,name,default_title,status,body,fields_json,revision FROM collection_templates
             WHERE collection_id=?1 AND id=?2", params![collection,id], read_template,
        ).optional()?.ok_or("這個集合沒有這份範本。")?;
        template.resources = resources::read(&self.connection, id)?;
        template.children = children::read(&self.connection, id)?;
        if let Some(snapshot) = snapshot { snapshot.commit()?; }
        Ok(template)
    }

    pub(crate) fn update_collection_template(
        &self, collection: i64, original: &CollectionTemplate, draft: &CollectionTemplate,
    ) -> Result<()> {
        let name = checked_label(&draft.name)?;
        let title = checked_title(&draft.default_title)?;
        let status = checked_label(&draft.status)?;
        validate_template_body(&draft.body)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        ensure_current(&tx, collection, original)?;
        validate_fields(&tx, collection, &draft.fields)?;
        require_available_name(&tx, collection, name, Some(original.id))?;
        tx.execute(
            "UPDATE collection_templates SET name=?3,default_title=?4,status=?5,body=?6,
             fields_json=?7,revision=revision+1 WHERE collection_id=?1 AND id=?2",
            params![collection, original.id, name, title, status, draft.body, encode_fields(&draft.fields)],
        )?;
        resources::retain(&tx, original.id, &draft.resources)?;
        children::replace(&tx, original.id, draft.body.len(), &draft.children)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn remove_collection_template(&self, collection: i64, template: &CollectionTemplate) -> Result<()> {
        let changed = self.connection.execute(
            "DELETE FROM collection_templates WHERE collection_id=?1 AND id=?2 AND revision=?3",
            params![collection, template.id, template.revision],
        )?;
        if changed != 1 { return Err("範本已更改或移除，請重新開啟。".into()); }
        Ok(())
    }

    pub(crate) fn instantiate_collection_template(
        &self, collection: i64, template: &CollectionTemplate, title: &str, options: TemplateApply,
    ) -> Result<i64> {
        let title = checked_title(title)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        super::super::obsidian_base_dynamic::require_manual_membership(&tx,collection)?;
        // Re-read inside the write transaction and copy only the saved recipe.
        let saved = self.collection_template(collection, template.id)?;
        if saved.revision != template.revision { return Err("範本已更改，請重新開啟預覽。".into()); }
        let status = checked_label(&saved.status)?;
        validate_template_body(&saved.body)?;
        validate_fields(&tx, collection, &saved.fields)?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM collection_notes WHERE collection_id=?1", [collection], |row| row.get(0),
        )?;
        if count >= MAX_ITEMS { return Err("集合最多 1000 篇筆記，含垃圾桶中的筆記。".into()); }
        let note = insert(&tx, None, title, &saved.body)?;
        for field in &saved.fields { write_value(&tx, note, &field.key, &field.value, true)?; }
        tx.execute("INSERT INTO collection_notes(collection_id,note_id,status) VALUES(?1,?2,?3)", params![collection, note, status])?;
        super::super::collection_field_rules::apply_on_admission(&tx, collection, note, None, false)?;
        if options.resources {
            resources::apply(&tx, collection, note, &saved.resources)?;
        }
        if options.children {
            children::apply(&tx, note, saved.body.len(), &saved.children)?;
        }
        tx.commit()?;
        Ok(note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::collection_views::{CollectionLayout, Month, Presentation};

    #[test]
    fn authored_nested_pages_preserve_paths_and_create_independent_hierarchy() -> Result<()> {
        use super::super::super::View;
        let dir = crate::store::tests::data_dir();
        let path = dir.join("workspace.sqlite3");
        let db = Notebook::open(&path, false)?;
        let collection = db.create_collection("空集合可撰寫巢狀範本")?;
        let mut draft = CollectionTemplate::blank("巢狀範本".into(),"預設根".into(),"待做".into(),"根正文".into());
        let page = |title: &str, body: &str, parent| TemplateChild { title:title.into(), body:body.into(), parent };
        draft.children = vec![
            page("第一層甲", "甲", None), page("第二層", "# 二層", Some(0)),
            page("第一層乙", "乙", None), page("另一個第二層", "# 另一層", Some(2)),
        ];
        let mut invalid = draft.clone();
        invalid.children[0].parent = Some(1);
        assert!(db.new_collection_template(collection, &invalid).is_err());
        assert!(db.collection_templates(collection)?.is_empty());
        let id = db.new_collection_template(collection, &draft)?;
        assert!(db.collection_rows(collection)?.is_empty());
        let current = db.collection_template(collection, id)?;
        assert_eq!(current.child_paths(), ["第一層甲","第一層甲 / 第二層","第一層乙","第一層乙 / 另一個第二層"]);
        let plain = db.instantiate_collection_template(collection, &current, "不含層", TemplateApply::default())?;
        let children = |db: &Notebook, parent: i64| -> Result<Vec<(i64,String)>> {
            Ok(db.rows(View::All,"")?.into_iter().filter(|row| row.parent == Some(parent))
                .map(|row| (row.id,row.title)).collect())
        };
        assert!(children(&db, plain)?.is_empty());
        let first = db.instantiate_collection_template(collection, &current, "含階層甲", TemplateApply { children:true, ..Default::default() })?;
        let first_level = children(&db, first)?;
        assert_eq!(first_level.iter().map(|(_,name)| name.as_str()).collect::<Vec<_>>(), ["第一層甲","第一層乙"]);
        let second_level = children(&db, first_level[0].0)?;
        assert_eq!(second_level[0].1, "第二層");
        drop(db);

        let db = Notebook::open(&path, false)?;
        let current = db.collection_template(collection,id)?;
        let second = db.instantiate_collection_template(collection, &current, "含階層乙", TemplateApply { children:true, ..Default::default() })?;
        let second_branch = children(&db, second)?;
        let second_repeat = children(&db, second_branch[0].0)?;
        assert_ne!(second_branch[0].0, first_level[0].0);
        assert_ne!(second_repeat[0].0, second_level[0].0);
        let old = db.note(second_repeat[0].0)?.unwrap();
        db.save(old.row.id,old.version,&old.row.title,"第二份可獨立編輯")?;
        assert_eq!(db.note(second_level[0].0)?.unwrap().body,"# 二層");
        let before = db.rows(View::All, "")?.len();
        db.connection.execute_batch("CREATE TEMP TRIGGER template_nested_abort BEFORE INSERT ON notes
            WHEN NEW.title='第二層' BEGIN SELECT RAISE(ABORT,'模擬子層建立失敗'); END;")?;
        assert!(db.instantiate_collection_template(collection, &current, "測試回滾", TemplateApply { children:true, ..Default::default() }).is_err());
        assert_eq!(db.rows(View::All, "")?.len(), before);
        db.connection.execute_batch("DROP TRIGGER template_nested_abort")?;

        let mut update = current.clone();
        assert_eq!(update.child_removal_count(0), 2);
        update.remove_child(0);
        update.children[0].title = "保有的分支".into();
        db.update_collection_template(collection, &current, &update)?;
        let saved = db.collection_template(collection, id)?;
        assert_eq!(saved.child_paths(), ["保有的分支","保有的分支 / 另一個第二層"]);
        assert_eq!(saved.children[1].parent, Some(0));
        let third = db.instantiate_collection_template(collection, &saved, "更新後再建立", TemplateApply { children:true, ..Default::default() })?;
        let third_branch = children(&db, third)?;
        assert_eq!(third_branch.len(),1);
        assert_eq!(children(&db, third_branch[0].0)?[0].1,"另一個第二層");
        assert_eq!(children(&db, first)?.len(),2);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn blank_recipe_authors_typed_defaults_without_requiring_source_rows() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("workspace.sqlite3");
        let db = Notebook::open(&path, false)?;
        let collection = db.create_collection("空的集合")?;
        let other = db.create_collection("另一個集合")?;
        for (key, kind) in [
            ("工時",PropertyType::Number), ("排程",PropertyType::Date), ("完成",PropertyType::Checkbox),
        ] {
            db.compare_set_property_type(key, None, Some(kind))?;
        }
        let mut draft = CollectionTemplate::blank("自行配置".into(), "新原稿".into(), "待處理".into(), "# 欄位與規格".into());
        draft.fields.push(TemplateField { key: "工時".into(), kind: PropertyType::Number, value: "0005.500".into() });
        draft.fields.push(TemplateField { key: "完成".into(), kind: PropertyType::Checkbox, value: "false".into() });
        draft.fields.push(TemplateField { key: "排程".into(), kind: PropertyType::Date, value: "2027-02-29".into() });
        assert!(db.new_collection_template(collection, &draft).is_err());
        assert!(db.collection_templates(collection)?.is_empty());
        draft.fields.pop();
        let id = db.new_collection_template(collection, &draft)?;
        assert!(db.new_collection_template(collection, &draft).is_err());
        db.new_collection_template(other, &draft)?;
        assert!(db.collection_rows(collection)?.is_empty());
        let template = db.collection_template(collection, id)?;
        let first = db.instantiate_collection_template(collection, &template, "第一筆", TemplateApply::default())?;
        assert_eq!(db.note(first)?.unwrap().body, "# 欄位與規格");
        let mut edit = template.clone();
        edit.fields.push(TemplateField { key: "排程".into(), kind: PropertyType::Date, value: "2028-02-29".into() });
        db.update_collection_template(collection, &template, &edit)?;
        assert!(db.instantiate_collection_template(collection, &template, "過期選擇", TemplateApply::default()).is_err());
        drop(db);

        let db = Notebook::open(&path, false)?;
        let template = db.collection_template(collection, id)?;
        assert!(template.children.is_empty() && template.resources.is_empty());
        assert_eq!(template.default_title, "新原稿");
        let second = db.instantiate_collection_template(collection, &template, &template.default_title, TemplateApply::default())?;
        assert_ne!(first,second);
        assert_eq!(db.note(second)?.unwrap().body, db.note(first)?.unwrap().body);
        assert_eq!(db.properties(first)?.len(), 2);
        let values = db.properties(second)?;
        assert_eq!(values.len(), 3);
        assert!(values.iter().any(|(key,value)| key == "排程" && value == "2028-02-29"));
        assert!(values.iter().any(|(key,value)| key == "工時" && value == "0005.500"));
        assert!(values.iter().any(|(key,value)| key == "完成" && value == "false"));
        assert_eq!(db.collection_rows(collection)?.len(), 2);
        assert!(db.collection_rows(other)?.is_empty());
        assert!(db.collection_rows(collection)?.iter().all(|(_,status)| status == "待處理"));
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn explicit_child_snapshots_get_independent_ids_and_rollback_with_other_choices() -> Result<()> {
        use super::super::super::View;
        let dir = crate::store::tests::data_dir();
        let path = dir.join("workspace.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let collection = db.create_collection("分鏡選取")?;
        let source = db.add(None, "源資料列", "# 範本正文")?;
        let child = db.add(Some(source), "選定的子頁", "[[不能帶入的參照]]")?;
        let unselected = db.add(Some(source), "不選的子頁", "留在來源")?;
        let deeper = db.add(Some(child), "未明確選擇的孫頁", "留在來源")?;
        db.add_to_collection(collection, source, "待辦")?;
        db.compare_set_property_type("工時", None, Some(PropertyType::Number))?;
        db.compare_set_property(source, "工時", None, Some("002.5"))?;
        db.compare_set_property(child, "工時", None, Some("7"))?;
        db.set_relation(source, "明確關聯", child, true)?;
        let file = dir.join("合成原始附件.txt");
        std::fs::write(&file, "獨立附件")?;
        let attachment = db.import_attachment(source, &file)?;
        let preview = db.collection_template_source(collection, RelationMember { note: source, status: "待辦" })?;
        assert_eq!(preview.children.iter().map(|(id,_)| *id).collect::<Vec<_>>(), [child,unselected]);
        assert!(db.collection_template_child_source(&preview, deeper).is_err());
        let draft = db.collection_template_child_source(&preview, child)?;
        assert!(db.capture_collection_template(collection, &preview, "連結仍未移除", &preview.body,
            TemplateSelection { children: &[draft], ..Default::default() }).is_err());
        let mut draft = db.collection_template_child_source(&preview, child)?;
        draft.content.body = "# 檢閱後的第一版".into();
        let old = db.note(child)?.unwrap();
        db.save(child, old.version, &old.row.title, "來源已更改")?;
        assert!(db.capture_collection_template(collection, &preview, "過期子頁", &preview.body,
            TemplateSelection { children: &[draft], ..Default::default() }).is_err());
        assert!(db.collection_templates(collection)?.is_empty());
        let mut draft = db.collection_template_child_source(&preview, child)?;
        draft.content.body = "# 已審閱的標題與清單\n\n- [ ] 檢查".into();
        let saved = db.capture_collection_template(collection, &preview, "可選子頁", &preview.body,
            TemplateSelection { fields: &[0], resources: &[0,1], children: &[draft] })?;
        let recipe = db.collection_template(collection, saved)?;
        assert_eq!(recipe.children.len(), 1);
        let child_content = recipe.children[0].body.clone();
        let tabs = |db: &Notebook, parent: i64| -> Result<Vec<i64>> {
            Ok(db.rows(View::All, "")?.into_iter().filter(|row| row.parent == Some(parent)).map(|row| row.id).collect())
        };
        let first = db.instantiate_collection_template(collection, &recipe, "無子頁", TemplateApply { resources: true, children: false })?;
        assert!(tabs(&db, first)?.is_empty());
        assert_eq!(db.relations(first)?[0].1.id, child);
        assert_ne!(db.list_attachments(first)?[0].id, attachment);
        drop(db);

        let db = Notebook::open(&path, false)?;
        let recipe = db.collection_template(collection, saved)?;
        let a = db.instantiate_collection_template(collection, &recipe, "新根甲", TemplateApply { children: true, ..Default::default() })?;
        let b = db.instantiate_collection_template(collection, &recipe, "新根乙", TemplateApply { resources: true, children: true })?;
        let a_children = tabs(&db,a)?;
        let b_children = tabs(&db,b)?;
        assert_eq!((a_children.len(),b_children.len()), (1,1));
        let (a_child,b_child) = (a_children[0], b_children[0]);
        assert_ne!(a_child,b_child);
        assert!(![child,unselected,deeper].contains(&a_child));
        assert_eq!(db.note(a_child)?.unwrap().body, child_content);
        assert_eq!(db.note(b_child)?.unwrap().body, child_content);
        assert!(tabs(&db,a_child)?.is_empty() && tabs(&db,b_child)?.is_empty());
        assert!(db.properties(a_child)?.is_empty());
        assert_eq!(db.properties(a)?[0].1, "002.5");
        assert!(db.list_attachments(a)?.is_empty());
        assert_eq!(db.relations(b)?[0].1.id, child);
        assert!(db.collection_rows(collection)?.iter().all(|(row,_)| row.id != a_child && row.id != b_child));
        let old = db.note(a_child)?.unwrap();
        db.save(a_child, old.version, &old.row.title, "新子頁可獨立修改")?;
        assert_eq!(db.note(b_child)?.unwrap().body, child_content);
        assert_eq!(db.note(child)?.unwrap().body, "來源已更改");

        let counts = |db: &Notebook| -> Result<(i64,i64,i64,i64,i64)> {
            Ok(db.connection.query_row("SELECT (SELECT count(*) FROM notes), (SELECT count(*) FROM note_properties),
                (SELECT count(*) FROM collection_notes), (SELECT count(*) FROM note_relations), (SELECT count(*) FROM attachments)",
                [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?)
        };
        let before = counts(&db)?;
        db.connection.execute_batch(
            "CREATE TEMP TRIGGER break_child_write BEFORE INSERT ON notes WHEN NEW.parent IS NOT NULL
             BEGIN SELECT RAISE(ABORT,'模擬子頁建立中斷'); END;"
        )?;
        assert!(db.instantiate_collection_template(collection, &recipe, "失敗時無根或附件", TemplateApply { resources: true, children: true }).is_err());
        assert_eq!(counts(&db)?, before);
        db.connection.execute_batch("DROP TRIGGER break_child_write")?;
        let mut removed = recipe.clone();
        removed.children.clear();
        db.update_collection_template(collection, &recipe, &removed)?;
        assert!(db.collection_template(collection, saved)?.children.is_empty());
        assert_eq!(tabs(&db,b)?, [b_child]);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn preview_accepts_markdown_and_refuses_live_or_embedded_links() {
        assert!(validate_template_body("# 摘要\n\n- [ ] **核對**\n\n`片段`").is_ok());
        for body in [
            "[指向](../私人.md)", "![[附件.pdf]]", "![](relative.png)", "<a href='thing'>連結</a>",
            "[別名]: ../私人.md", "看 file:///磁碟/秘密.md", "標籤 https://example.invalid/private",
        ] {
            assert!(validate_template_body(body).is_err(), "{body}");
        }
    }

    #[test]
    fn template_resource_opt_in_is_selective_live_and_atomic() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("workspace.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let collection = db.create_collection("可重用資料列")?;
        let source = db.add(None, "來源", "# 正文")?;
        let target = db.add(None, "同名對象", "")?;
        let unselected_target = db.add(None, "同名對象", "")?;
        db.add_to_collection(collection, source, "排程")?;
        db.set_relation(source, "參考", target, true)?;
        db.set_relation(source, "參考", unselected_target, true)?;
        let fixture = dir.join("已選附件.pdf");
        let pdf = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
        std::fs::write(&fixture, pdf)?;
        let selected_attachment = db.import_attachment(source, &fixture)?;
        db.add_pdf_annotation(selected_attachment, 1, "只留原始註解", "不套用")?;
        let spare = dir.join("留在來源.txt");
        std::fs::write(&spare, "不選擇這份合成附件")?;
        db.import_attachment(source, &spare)?;
        let row = db.collection_template_source(collection, RelationMember { note: source, status: "排程" })?;
        assert_eq!(row.resources.len(), 4);
        let relation_index = row.resources.iter().position(|resource| resource.label().contains(&format!("(#{target})"))).unwrap();
        let file_index = row.resources.iter().position(|resource| resource.label().contains("已選附件.pdf")).unwrap();
        assert!(db.capture_collection_template(collection, &row, "同一項勿重複", &row.body, TemplateSelection { resources: &[relation_index, relation_index], ..Default::default() }).is_err());
        assert!(db.collection_templates(collection)?.is_empty());
        let recipe = db.capture_collection_template(collection, &row, "可選套用", &row.body, TemplateSelection { resources: &[relation_index, file_index], ..Default::default() })?;
        let stored = db.collection_template(collection, recipe)?;
        assert_eq!(stored.resources.len(), 2);
        let plain = db.instantiate_collection_template(collection, &stored, "複本甲", TemplateApply::default())?;
        assert!(db.relations(plain)?.is_empty());
        assert!(db.list_attachments(plain)?.is_empty());
        drop(db);

        let mut db = Notebook::open(&path, false)?;
        let old = db.note(target)?.unwrap();
        db.save(target, old.version, "改名的同一目標", &old.body)?;
        let stored = db.collection_template(collection, recipe)?;
        assert!(stored.resources[0].label().contains("改名的同一目標"));
        let copy = db.instantiate_collection_template(collection, &stored, "複本乙", TemplateApply { resources: true, ..Default::default() })?;
        assert_eq!(db.relations(copy)?.iter().map(|(_, note)| note.id).collect::<Vec<_>>(), [target]);
        let applied = db.list_attachments(copy)?;
        assert_eq!(applied.len(), 1);
        assert_ne!(applied[0].id, selected_attachment);
        assert_eq!(db.read_pdf_attachment(applied[0].id)?.1.as_slice(), pdf);
        assert!(db.list_pdf_annotations(applied[0].id)?.is_empty());
        db.remove_attachment(applied[0].id)?;
        assert_eq!(db.list_attachments(source)?.len(), 2);
        assert_eq!(db.list_pdf_annotations(selected_attachment)?.len(), 1);
        assert_eq!(db.relations(source)?.len(), 2);

        let totals = |db: &Notebook| -> Result<(i64,i64,i64,i64)> {
            Ok(db.connection.query_row(
                "SELECT (SELECT count(*) FROM notes), (SELECT count(*) FROM collection_notes),
                    (SELECT count(*) FROM note_relations), (SELECT count(*) FROM attachments)",
                [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            )?)
        };
        let before = totals(&db)?;
        db.connection.execute_batch(
            "CREATE TEMP TRIGGER break_resource_copy BEFORE INSERT ON attachments
             WHEN NEW.name='已選附件.pdf' BEGIN SELECT RAISE(ABORT,'測試寫入中斷'); END;",
        )?;
        assert!(db.instantiate_collection_template(collection, &stored, "附件失敗時無殘留", TemplateApply { resources: true, ..Default::default() }).is_err());
        assert_eq!(totals(&db)?, before);
        db.connection.execute_batch("DROP TRIGGER break_resource_copy")?;
        db.set_relation(source, "參考", target, false)?;
        let no_relation = totals(&db)?;
        assert!(db.instantiate_collection_template(collection, &stored, "不能無聲跳過失效關聯", TemplateApply { resources: true, ..Default::default() }).is_err());
        assert_eq!(totals(&db)?, no_relation);
        db.set_relation(source, "參考", target, true)?;

        db.remove_attachment(selected_attachment)?;
        let replacement = db.import_attachment(source, &fixture)?;
        assert_ne!(replacement, selected_attachment);
        let stale = db.collection_template(collection, recipe)?;
        assert!(stale.resources.iter().any(|resource| resource.label().contains("找不到原檔")));
        let before = totals(&db)?;
        assert!(db.instantiate_collection_template(collection, &stale, "同名同位元仍是不同附件", TemplateApply { resources: true, ..Default::default() }).is_err());
        assert_eq!(totals(&db)?, before);
        let plain = db.instantiate_collection_template(collection, &stale, "第三筆僅正文", TemplateApply::default())?;
        assert!(db.relations(plain)?.is_empty());
        assert!(db.list_attachments(plain)?.is_empty());

        let mut repair = stale.clone();
        repair.resources.retain(|resource| !resource.label().contains("找不到原檔"));
        db.update_collection_template(collection, &stale, &repair)?;
        let repaired = db.collection_template(collection, recipe)?;
        let recovered = db.instantiate_collection_template(collection, &repaired, "已移除失效項目", TemplateApply { resources: true, ..Default::default() })?;
        assert_eq!(db.relations(recovered)?[0].1.id, target);
        assert!(db.list_attachments(recovered)?.is_empty());
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn v17_row_yields_independent_scope_limited_templates_without_partial_creation() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("workspace.sqlite3");
        let (collection, other, source, target, child) = {
            let mut db = Notebook::open(&path, false)?;
            let collection = db.create_collection("分鏡")?;
            let other = db.create_collection("剪輯")?;
            let source = db.add(None, "來源筆記", "# 源\n\n[[私人資料]]")?;
            let target = db.add(None, "私人資料", "")?;
            let child = db.add(Some(source), "舊子頁", "")?;
            db.add_to_collection(collection, source, "待拍")?;
            db.set_relation(source, "參考", target, true)?;
            let attachment = dir.join("剪輯範圍.pdf");
            std::fs::write(&attachment, include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf"))?;
            db.import_attachment(source, &attachment)?;
            for (key, kind, raw) in [
                ("工時", PropertyType::Number, "002.500"),
                ("確認", PropertyType::Checkbox, " TRUE "),
                ("開始", PropertyType::Date, "2028-02-28"),
                ("結束", PropertyType::Date, "2028-03-01"),
            ] {
                db.compare_set_property_type(key, None, Some(kind))?;
                db.compare_set_property(source, key, None, Some(raw))?;
            }
            db.compare_set_property(source, "備註", None, Some("只留在來源"))?;
            db.save_presented_collection_view(collection, "排程", CollectionLayout::Table, "", "", None,
                Presentation { calendar_key: Some("開始".into()), calendar_end_key: Some("結束".into()),
                    timeline_mode: true, calendar_month: Month::parse("2028-02"), ..Default::default() })?;
            crate::store::tests::remove_v18_schema(&db.connection)?;
            db.connection.pragma_update(None, "user_version", 17)?;
            (collection, other, source, target, child)
        };
        let db = Notebook::open(&path, false)?;
        assert!(db.collection_templates(collection)?.is_empty());
        let member = RelationMember { note: source, status: "待拍" };
        let candidate = db.collection_template_source(collection, member)?;
        let fields = |preview: &TemplateSource, key: &str| preview.fields.iter().position(|field| field.key == key).unwrap();
        assert_eq!(candidate.fields.len(), 4);
        let hours = fields(&candidate, "工時");
        let end = fields(&candidate, "結束");
        assert!(db.capture_collection_template(other, &candidate, "跨集合", "", TemplateSelection::default()).is_err());
        assert!(db.capture_collection_template(collection, &candidate, "含舊連結", &candidate.body, TemplateSelection::default()).is_err());
        assert!(db.capture_collection_template(collection, &candidate, "缺起始", "", TemplateSelection { fields: &[end], ..Default::default() }).is_err());
        db.compare_set_property(source, "工時", Some("002.500"), Some("3"))?;
        assert!(db.capture_collection_template(collection, &candidate, "過期欄位", "", TemplateSelection { fields: &[hours], ..Default::default() }).is_err());
        let candidate = db.collection_template_source(collection, member)?;
        db.compare_set_property(source, "工時", Some("3"), Some("002.500"))?;
        assert!(db.capture_collection_template(collection, &candidate, "過期欄位", "", TemplateSelection { fields: &[hours], ..Default::default() }).is_err());
        let candidate = db.collection_template_source(collection, member)?;
        let body = "# 分鏡\n\n- [ ] 先看原稿";
        let id = db.capture_collection_template(collection, &candidate, "鏡頭", body, TemplateSelection { fields: &[hours, fields(&candidate,"確認")], ..Default::default() })?;
        assert!(db.capture_collection_template(collection, &candidate, " 鏡頭 ", "", TemplateSelection::default()).is_err());
        let recipe = db.collection_template(collection, id)?;
        assert!(db.instantiate_collection_template(other, &recipe, "不能跨集合", TemplateApply::default()).is_err());
        assert!(db.instantiate_collection_template(collection, &recipe, " ", TemplateApply::default()).is_err());
        let a = db.instantiate_collection_template(collection, &recipe, "第一筆", TemplateApply::default())?;
        let b = db.instantiate_collection_template(collection, &recipe, "第二筆", TemplateApply::default())?;
        assert_ne!(a, b);
        assert!(![source,target,child].contains(&a) && ![source,target,child].contains(&b));
        for note in [a, b] {
            let stored = db.note(note)?.unwrap();
            assert_eq!(stored.body, body);
            assert_eq!(stored.row.parent, None);
            assert_eq!(db.properties(note)?, [("工時".into(), "002.500".into()), ("確認".into(), " TRUE ".into())]);
            assert!(db.relations(note)?.is_empty());
            assert!(db.list_attachments(note)?.is_empty());
        }
        assert_eq!(db.relations(source)?[0].1.id, target);
        assert_eq!(db.list_attachments(source)?.len(), 1);
        assert_eq!(db.note(child)?.unwrap().row.parent, Some(source));
        assert!(db.note(source)?.unwrap().body.contains("[[私人資料]]"));
        assert!(db.collection_rows(collection)?.iter().all(|(_,status)| status == "待拍"));
        db.compare_set_property(a, "工時", Some("002.500"), Some("7"))?;
        let first = db.note(a)?.unwrap();
        db.save(a, first.version, &first.row.title, "獨立修改")?;
        assert_eq!(db.note(b)?.unwrap().body, body);
        drop(db);

        let db = Notebook::open(&path, false)?;
        let recipe = db.collection_template(collection, id)?;
        assert_eq!(db.note(a)?.unwrap().body, "獨立修改");
        assert_eq!(db.note(b)?.unwrap().body, body);
        assert_eq!(db.properties(b)?[0].1, "002.500");
        let mut edited = recipe.clone();
        edited.default_title = "新版預設".into();
        edited.status = "已排".into();
        edited.body = "新版正文".into();
        db.update_collection_template(collection, &recipe, &edited)?;
        assert!(db.instantiate_collection_template(collection, &recipe, "過期範本", TemplateApply::default()).is_err());
        assert!(db.remove_collection_template(collection, &recipe).is_err());

        let latest = db.collection_template(collection, id)?;
        let counts = |db: &Notebook| -> Result<(i64, i64, i64)> {
            Ok(db.connection.query_row("SELECT (SELECT count(*) FROM notes),
                (SELECT count(*) FROM note_properties),(SELECT count(*) FROM collection_notes)", [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?)
        };
        let before = counts(&db)?;
        db.connection.execute_batch("CREATE TEMP TRIGGER refuse_test_template_member BEFORE INSERT ON collection_notes
            WHEN NEW.status='已排' BEGIN SELECT RAISE(ABORT,'測試失敗'); END;")?;
        assert!(db.instantiate_collection_template(collection, &latest, "寫入中斷", TemplateApply::default()).is_err());
        assert_eq!(counts(&db)?, before);
        db.connection.execute_batch("DROP TRIGGER refuse_test_template_member;")?;
        db.compare_set_property_type("工時", Some(PropertyType::Number), Some(PropertyType::Date))?;
        assert!(db.instantiate_collection_template(collection, &latest, "型別改變", TemplateApply::default()).is_err());
        assert_eq!(counts(&db)?, before);
        db.compare_set_property_type("工時", Some(PropertyType::Date), Some(PropertyType::Number))?;
        let c = db.instantiate_collection_template(collection, &latest, &latest.default_title, TemplateApply::default())?;
        assert_eq!(db.note(c)?.unwrap().body, "新版正文");
        db.remove_collection_template(collection, &latest)?;
        assert!(db.collection_templates(collection)?.is_empty());
        assert_eq!(db.note(b)?.unwrap().body, body);
        assert_eq!(db.note(c)?.unwrap().row.title, "新版預設");
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
