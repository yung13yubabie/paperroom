//! Collection admission rules for the workspace-wide typed note properties.
use super::{Notebook, Result};
use super::collection_views::{CalendarDay, validate_collection_field_dates};
use super::properties::{MAX_PROPERTIES, MAX_VALUE_BYTES, key_checked, write_value};
use super::typed_properties::{PropertyType, lookup};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub(super) const SCHEMA: &str = "
CREATE TABLE collection_field_rules (
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    key TEXT NOT NULL REFERENCES property_definitions(key),
    required INTEGER NOT NULL CHECK(required IN (0,1)),
    default_value TEXT,
    PRIMARY KEY(collection_id,key),
    CHECK(required=1 OR default_value IS NOT NULL)
);
CREATE INDEX collection_field_rules_by_key ON collection_field_rules(key);
CREATE INDEX collection_notes_by_note ON collection_notes(note_id,collection_id);
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CollectionFieldRule {
    pub key: String,
    pub kind: PropertyType,
    pub required: bool,
    pub default: Option<String>,
}

pub(crate) struct CollectionRuleInput<'a> {
    pub rules: &'a [CollectionFieldRule],
    pub values: &'a [RuleValue],
}

pub(crate) enum RuleValue {
    Existing(String),
    Input(String),
}

fn rules(db: &Connection, collection: i64) -> Result<Vec<CollectionFieldRule>> {
    let mut stmt = db.prepare(
        "SELECT r.key,d.kind,r.required,r.default_value FROM collection_field_rules r
         JOIN property_definitions d ON d.key=r.key WHERE r.collection_id=?1 ORDER BY r.key",
    )?;
    Ok(stmt.query_map([collection], |row| Ok(CollectionFieldRule {
        key: row.get(0)?,
        kind: PropertyType::decode(row.get_ref(1)?.as_str()?)?,
        required: row.get(2)?,
        default: row.get(3)?,
    }))?.collect::<rusqlite::Result<_>>()?)
}

fn validate_typed(key: &str, kind: PropertyType, value: &str) -> Result<()> {
    if value.len() > MAX_VALUE_BYTES || value.contains('\0') || kind.normalized(value).is_none() {
        return Err(format!("「{key}」需要有效的{}，且不得超過 4096 位元組。", kind.hint()).into());
    }
    Ok(())
}

/// Validate a form value without altering legacy text or applying a fallback to explicit blanks.
pub(crate) fn validate_rule_input(rule: &CollectionFieldRule, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        if rule.required { return Err(format!("「{}」是集合必填欄位，請先填寫。", rule.key).into()); }
    } else {
        validate_typed(&rule.key, rule.kind, value)?;
    }
    Ok(())
}

impl Notebook {
    pub(crate) fn collection_field_rules(&self, collection: i64) -> Result<Vec<CollectionFieldRule>> {
        rules(&self.connection, collection)
    }

    pub(crate) fn save_collection_field_rule(
        &self, collection: i64, rule: &CollectionFieldRule, expected: Option<&CollectionFieldRule>,
    ) -> Result<()> {
        let key = key_checked(&rule.key)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let current = rules(&tx, collection)?;
        if lookup(&tx, key)? != Some(rule.kind)
            || current.iter().find(|item| item.key == key) != expected
        {
            return Err("欄位規則或型別已由其他視窗更改，請重新開啟。".into());
        }
        if let Some(value) = rule.default.as_deref() {
            validate_typed(key, rule.kind, value)?;
        }
        if !rule.required && rule.default.is_none() {
            tx.execute("DELETE FROM collection_field_rules WHERE collection_id=?1 AND key=?2", params![collection,key])?;
        } else {
            if expected.is_none() && current.len() >= MAX_PROPERTIES as usize {
                return Err("每個集合最多 128 項欄位規則。".into());
            }
            let inserted = tx.execute(
                "INSERT INTO collection_field_rules(collection_id,key,required,default_value)
                 SELECT id,?2,?3,?4 FROM collections WHERE id=?1
                 ON CONFLICT(collection_id,key) DO UPDATE SET required=excluded.required,default_value=excluded.default_value",
                params![collection,key,rule.required,rule.default],
            )?;
            if inserted != 1 { return Err("找不到集合。".into()); }
        }
        tx.commit()?;
        Ok(())
    }
}

pub(super) fn apply_on_admission(
    tx: &Transaction<'_>, collection: i64, note: i64, input: Option<CollectionRuleInput<'_>>,
    mut track_history: bool,
) -> Result<()> {
    let rules = rules(tx, collection)?;
    if input.as_ref().is_some_and(|form| form.rules != rules || form.values.len() != rules.len()) {
        return Err("集合欄位規則已變更，請重新開啟。".into());
    }
    let mut dates = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        let key = rule.key.as_str();
        let previous: Option<String> = tx.query_row(
            "SELECT value FROM note_properties WHERE note_id=?1 AND key=?2",
            params![note,key], |row| row.get(0),
        ).optional()?;
        let entered = input.as_ref().map(|form| &form.values[index]);
        let value = match (entered, previous.as_deref()) {
            (Some(RuleValue::Existing(expected)), Some(current)) if current == expected => current,
            (Some(RuleValue::Input(value)), None) => value,
            (None, Some(current)) => current,
            (None, None) => rule.default.as_deref().unwrap_or(""),
            _ => return Err(format!("「{key}」已由其他視窗變更，請重新開啟。").into()),
        };
        if previous.is_none() || rule.required {
            validate_rule_input(rule, value)?;
        }
        if previous.is_none() && !value.trim().is_empty() {
            if track_history {
                super::note_history_state::checkpoint(tx, note)?;
                track_history = false;
            }
            write_value(tx, note, key, value, true)?;
        }
        if rule.kind == PropertyType::Date {
            dates.push((key, CalendarDay::parse(value)));
        }
    }
    // Admission may supply multiple date fields at once. Validate their final
    // values, including the newly joined collection and any other memberships.
    for date in dates {
        validate_collection_field_dates(tx, note, [date, ("", None)])?;
    }
    Ok(())
}

pub(super) fn ensure_clear_allowed(db: &Connection, note: i64, key: &str) -> Result<()> {
    let collection: Option<String> = db.query_row(
        "SELECT c.name FROM collection_notes m
         JOIN collection_field_rules r ON r.collection_id=m.collection_id
         JOIN collections c ON c.id=m.collection_id
         WHERE m.note_id=?1 AND r.key=?2 AND r.required=1 ORDER BY c.id LIMIT 1",
        params![note,key], |row| row.get(0),
    ).optional()?;
    if let Some(collection) = collection {
        return Err(format!("「{key}」是集合「{collection}」的必填欄位，不能清除。").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{View, collections::CollectionEntry};

    fn set_rule(db: &Notebook, collection: i64, key: &str, kind: PropertyType, required: bool, default: Option<&str>) -> Result<()> {
        db.save_collection_field_rule(collection, &CollectionFieldRule {
            key: key.into(), kind, required, default: default.map(str::to_owned),
        }, None)
    }

    #[test]
    fn rules_admit_rows_atomically_and_do_not_backfill_or_replace_prior_values() -> Result<()> {
        let mut db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let collection = db.create_collection("分鏡")?;
        let second = db.create_collection("另一集合")?;
        let legacy = db.add(None, "舊筆記", "保留")?;
        db.add_to_collection(collection, legacy, "待處理")?;
        let existing = db.add(None, "已有屬性", "")?;
        db.compare_set_property(existing, "核取", None, Some("true"))?;
        db.compare_set_property_type("核取", None, Some(PropertyType::Checkbox))?;
        db.compare_set_property_type("週期", None, Some(PropertyType::Number))?;
        set_rule(&db, collection, "核取", PropertyType::Checkbox, true, Some("false"))?;
        set_rule(&db, collection, "週期", PropertyType::Number, true, None)?;
        assert!(db.properties(legacy)?.is_empty());
        assert!(db.add_to_collection(collection, existing, "待處理").is_err());
        assert_eq!(db.properties(existing)?, [("核取".into(), "true".into())]);
        let rules = db.collection_field_rules(collection)?;
        assert_eq!(rules.iter().map(|rule| rule.key.as_str()).collect::<Vec<_>>(), ["核取", "週期"]);

        let invalid = [RuleValue::Input("false".into()), RuleValue::Input(" ".into())];
        let form = |values| Some(CollectionRuleInput { rules: &rules, values });
        assert!(db.add_collection_entry(collection, CollectionEntry::New("不應出現"), "待處理", form(&invalid)).is_err());
        assert!(db.rows(View::All, "不應出現")?.is_empty());
        assert_eq!(db.collection_rows(collection)?.len(), 1);

        let valid = [RuleValue::Input("false".into()), RuleValue::Input("0".into())];
        let row = db.add_collection_entry(collection, CollectionEntry::New("新增"), "待處理", form(&valid))?;
        assert_eq!(db.properties(row)?, [("核取".into(),"false".into()),("週期".into(),"0".into())]);
        db.add_to_collection(second, row, "待處理")?;
        assert!(db.compare_set_property(row, "週期", Some("0"), None).is_err());
        assert!(db.compare_set_property(row, "週期", Some("0"), Some("無效")).is_err());
        let bad = crate::store::collection_views::FieldEdit {
            key: "週期", kind: PropertyType::Number, previous: Some("0"), value: None,
        };
        assert!(db.edit_collection_field(second, row, "待處理", bad).is_err());
        assert_eq!(db.properties(row)?[1].1, "0");
        db.compare_set_property(row, "週期", Some("0"), Some("2"))?;

        let saved = [RuleValue::Existing("true".into()), RuleValue::Input("0".into())];
        db.compare_set_property(existing,"核取",Some("true"),None)?;
        assert!(db.add_collection_entry(collection, CollectionEntry::Existing(existing), "待處理", form(&saved)).is_err());
        assert!(db.properties(existing)?.is_empty());
        db.compare_set_property(existing,"核取",None,Some("true"))?;
        db.add_collection_entry(collection, CollectionEntry::Existing(existing), "待處理", form(&saved))?;
        assert_eq!(db.properties(existing)?[0].1, "true");
        assert!(db.compare_set_property_type("核取", Some(PropertyType::Checkbox), Some(PropertyType::Date)).is_err());
        Ok(())
    }

    #[test]
    fn admission_checks_existing_date_pairs_and_two_date_edits_roll_back() -> Result<()> {
        use crate::store::collection_views::{CollectionLayout, Presentation};
        let mut db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let collection = db.create_collection("排程")?;
        for key in ["開始", "結束"] {
            db.compare_set_property_type(key, None, Some(PropertyType::Date))?;
        }
        let view = db.save_presented_collection_view(collection, "日期", CollectionLayout::Table, "", "", None, Presentation {
            calendar_key: Some("開始".into()), calendar_end_key: Some("結束".into()), ..Default::default()
        })?;
        set_rule(&db, collection, "開始", PropertyType::Date, false, Some("2028-03-01"))?;
        set_rule(&db, collection, "結束", PropertyType::Date, true, Some("2028-03-02"))?;

        let note = db.add(None, "既有日期", "")?;
        db.compare_set_property(note,"開始",None,Some("2028-03-20"))?;
        db.compare_set_property(note,"結束",None,Some("2028-03-10"))?;
        assert!(db.add_to_collection(collection,note,"待處理").is_err());
        assert!(db.collection_rows(collection)?.is_empty());
        assert!(db.properties(note)?.iter().any(|(key,value)| key=="開始" && value=="2028-03-20"));

        let new = db.add_collection_entry(collection,CollectionEntry::New("有效日期"),"待處理",None)?;
        let previous = [Some("2028-03-01"),Some("2028-03-02")];
        assert!(db.set_collection_row_dates(collection,&view,new,previous,[None,None]).is_err());
        for invalid in [Some("2028-03-03"), None] {
            assert!(db.compare_set_property(new,"開始",previous[0],invalid).is_err());
        }
        assert_eq!(db.properties(new)?, [("結束".into(),"2028-03-02".into()),("開始".into(),"2028-03-01".into())]);
        db.compare_set_property(new,"開始",previous[0],Some("2028-02-29"))?;
        Ok(())
    }

    #[test]
    fn migrates_version_29_and_restores_rules_and_values_from_one_backup() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let mut old = Notebook::open(&path, false)?;
        let note = old.add(None, "舊筆記", "原稿")?;
        let collection = old.create_collection("舊集合")?;
        old.add_to_collection(collection, note, "待處理")?;
        old.compare_set_property_type("日期", None, Some(PropertyType::Date))?;
        crate::store::tests::remove_v29_schema(&old.connection)?;
        old.connection.execute_batch("PRAGMA user_version=29; CREATE TABLE collection_field_rules (wrong_column TEXT);")?;
        drop(old);
        assert!(Notebook::open(&path,false).is_err());
        let legacy = Connection::open(&path)?;
        let version: i32 = legacy.pragma_query_value(None,"user_version", |row| row.get(0))?;
        assert_eq!(version,29);
        legacy.execute_batch("DROP TABLE collection_field_rules;")?;
        drop(legacy);
        let db = Notebook::open(&path, false)?;
        assert!(db.collection_field_rules(collection)?.is_empty());
        assert!(db.properties(note)?.is_empty());
        assert_eq!(db.note(note)?.unwrap().body, "原稿");
        set_rule(&db, collection, "日期", PropertyType::Date, true, Some("2028-02-29"))?;
        let expected = db.collection_field_rules(collection)?;
        let mut invalid = expected[0].clone();
        invalid.default = Some("2026-02-29".into());
        assert!(db.save_collection_field_rule(collection,&invalid,Some(&expected[0])).is_err());
        assert!(db.save_collection_field_rule(collection,&expected[0],None).is_err());

        let backup = dir.join("備份.sqlite3");
        db.backup(&backup)?;
        let restored = Notebook::open(&backup, false)?;
        assert_eq!(restored.collection_field_rules(collection)?, expected);
        assert!(restored.properties(note)?.is_empty());
        let new = restored.add_collection_entry(collection, CollectionEntry::New("還原新增"), "待處理", None)?;
        assert_eq!(restored.properties(new)?, [("日期".into(),"2028-02-29".into())]);
        assert_eq!(db.collection_rows(collection)?.len(), 1);
        drop(restored);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
