//! Evaluate one row against a single SQLite read snapshot. Results are never written to notes.
use super::{Notebook, Result, typed_properties::PropertyType};
use crate::formula::{CompiledFormula, Value};
use rusqlite::params;
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;

type Cached<T> = std::result::Result<T, String>;

/// One row evaluator lives for one view scan. Database reads share the caller's snapshot.
pub(super) struct DerivedRow<'a> {
    db: &'a Notebook,
    note: i64,
    types: &'a HashMap<String, PropertyType>,
    properties: OnceCell<Cached<HashMap<String, String>>>,
    formulas: RefCell<HashMap<String, Cached<Value>>>,
    membership: RefCell<HashMap<(String, i64), Cached<bool>>>,
    counts: RefCell<HashMap<String, Cached<usize>>>,
    sums: RefCell<HashMap<(String, String), Cached<f64>>>,
}

fn cached<K: Eq + std::hash::Hash, T: Clone>(
    cache: &RefCell<HashMap<K, Cached<T>>>, key: K, fetch: impl FnOnce() -> Result<T>,
) -> Cached<T> {
    cache.borrow_mut().entry(key).or_insert_with(|| fetch().map_err(|error| error.to_string())).clone()
}

impl<'a> DerivedRow<'a> {
    pub fn new(db: &'a Notebook, note: i64, types: &'a HashMap<String, PropertyType>) -> Self {
        Self { db, note, types, properties: OnceCell::new(), formulas: RefCell::default(),
            membership: RefCell::default(), counts: RefCell::default(), sums: RefCell::default() }
    }

    fn fields(&self) -> Result<&HashMap<String, String>> {
        self.properties.get_or_init(|| self.db.properties(self.note).map(|values| values.into_iter().collect())
            .map_err(|error| error.to_string())).as_ref().map_err(|error| error.clone().into())
    }

    pub fn relation_has(&self, name: &str, target: i64) -> Cached<bool> {
        cached(&self.membership, (name.to_owned(), target), || {
            let mut statement = self.db.connection.prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM note_relations r JOIN notes n ON n.id=r.target_id
                 WHERE r.source_id=?1 AND r.name=?2 AND r.target_id=?3 AND n.trash_root IS NULL)",
            )?;
            Ok(statement.query_row(params![self.note, name, target], |row| row.get(0))?)
        })
    }

    pub fn count(&self, name: &str) -> Cached<usize> {
        cached(&self.counts, name.to_owned(), || self.db.rollup_count(self.note, name))
    }

    pub fn sum(&self, name: &str, field: &str) -> Cached<f64> {
        cached(&self.sums, (name.to_owned(), field.to_owned()), || self.db.rollup_sum(self.note, name, field))
    }

    pub fn formula(&self, name: &str, code: &CompiledFormula) -> Cached<Value> {
        cached(&self.formulas, name.to_owned(), || self.evaluate(code))
    }

    fn evaluate(&self, code: &CompiledFormula) -> Result<Value> {
        let note = self.note;
        code.evaluate(|name, args| {
            let key = &args[0];
            match name {
                "field_count" | "field_sum" => self.db.relation_field_rollup(
                    self.note, key, &args[1], args.get(2).map(String::as_str),
                ).map(Value::Num),
                "relation_count" => self.count(key).map(|n| Value::Num(n as f64)).map_err(Into::into),
                "relation_sum" => self.sum(key, &args[1]).map(Value::Num).map_err(Into::into),
                "prop" | "typed" => {
                    let raw = self.fields()?.get(key).ok_or_else(|| format!("筆記 #{note} 缺少屬性「{key}」。"))?;
                    if raw.len() > super::MAX_VALUE_BYTES { return Err(format!("屬性「{key}」超過長度上限。").into()); }
                    if name == "prop" { return Ok(Value::Str(raw.clone())); }
                    let kind = self.types.get(key).copied()
                        .ok_or_else(|| format!("請先定義「{key}」的型別，或用 prop 讀取文字。"))?;
                    let normalized = kind.normalized(raw)
                        .ok_or_else(|| format!("請修正筆記 #{note} 的「{key}」為{}。", kind.hint()))?;
                    match kind {
                        PropertyType::Checkbox => Ok(Value::Bool(normalized == "true")),
                        PropertyType::Date => Ok(Value::Date(normalized)),
                        PropertyType::Number => {
                            let value: f64 = normalized.parse()?;
                            if !value.is_finite() { return Err(format!("「{key}」超出公式數值範圍。").into()); }
                            Ok(Value::Num(value))
                        }
                    }
                }
                _ => Err("未知公式函式。".into()),
            }
        })
    }
}

impl Notebook {
    pub(crate) fn evaluate_note_formula(&self, note: i64, expr: &str) -> Result<String> {
        let snapshot = self.connection.is_autocommit().then(|| self.connection.unchecked_transaction()).transpose()?;
        let types = self.property_definitions()?.into_iter().collect();
        let row = DerivedRow::new(self, note, &types);
        row.fields()?;
        let result = row.evaluate(&CompiledFormula::new(expr)?)?.into_display();
        if let Some(snapshot) = snapshot { snapshot.commit()?; }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_formulas_read_current_types_and_directed_targets() -> Result<()> {
        let mut db = Notebook::open(std::path::Path::new(":memory:"),false)?;
        let src = db.add(None,"同名","")?;
        let target = db.add(None,"同名","")?;
        let c = db.create_collection("工作")?;
        db.add_to_collection(c,src,"追蹤")?;
        for (key,kind,raw) in [("完成",PropertyType::Checkbox," TRUE "),("截止",PropertyType::Date,"2028-02-29"),("工時",PropertyType::Number,"002.50")] {
            db.compare_set_property_type(key,None,Some(kind))?;
            db.compare_set_property(src,key,None,Some(raw))?;
        }
        db.set_relation(src,"子項",target,true)?;
        db.compare_set_property(target,"工時",None,Some("4"))?;
        let expr = r#"if(typed("完成"),typed("工時")+relation_sum("子項","工時"),0)"#;
        db.add_collection_formula(c,"總工時",expr)?;
        let formulas = db.collection_formulas(c)?;
        assert_eq!(db.evaluate_note_formula(src,&formulas[0].1)?,"6.5");
        assert_eq!(db.evaluate_note_formula(src,r#"prop("工時")"#)?,"002.50");
        assert_eq!(db.evaluate_note_formula(src,r#"typed("截止") < date("2028-03-01")"#)?,"true");
        assert!(db.evaluate_note_formula(src,r#"typed("截止") == "2028-02-29""#).is_err());
        assert_eq!(db.evaluate_note_formula(target,r#"relation_count("子項")"#)?,"0");
        db.compare_set_property(target,"工時",Some("4"),Some("6"))?;
        assert_eq!(db.evaluate_note_formula(src,expr)?,"8.5");
        db.trash(target)?;
        assert_eq!(db.evaluate_note_formula(src,expr)?,"2.5");
        db.restore(target)?;
        assert_eq!(db.evaluate_note_formula(src,expr)?,"8.5");
        Ok(())
    }
}
