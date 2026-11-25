//! Workspace-wide type declarations validate edits while keeping original note text.
use super::{Notebook, Result};
use super::properties::key_checked;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub(super) const SCHEMA: &str = "
CREATE TABLE property_definitions (
 key TEXT PRIMARY KEY NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('number','checkbox','date'))
);
ALTER TABLE collection_views ADD COLUMN filter_key TEXT;
ALTER TABLE collection_views ADD COLUMN filter_kind TEXT;
ALTER TABLE collection_views ADD COLUMN filter_value TEXT;
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropertyType { Number, Checkbox, Date }

impl PropertyType {
    pub(crate) fn code(self) -> &'static str {
        match self { Self::Number => "number", Self::Checkbox => "checkbox", Self::Date => "date" }
    }

    pub(crate) fn decode(raw: &str) -> rusqlite::Result<Self> {
        match raw {
            "number" => Ok(Self::Number), "checkbox" => Ok(Self::Checkbox), "date" => Ok(Self::Date),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }

    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Number => "數字（整數或小數，例如 -12.50）",
            Self::Checkbox => "核取方塊（true 或 false）",
            Self::Date => "日期（YYYY-MM-DD，例如 2028-02-29）",
        }
    }

    pub(crate) fn normalized(self, raw: &str) -> Option<String> {
        let value = raw.trim();
        match self {
            Self::Number => {
                let digits = value.strip_prefix(['+','-']).unwrap_or(value);
                let (whole, fraction) = digits.split_once('.').unwrap_or((digits,""));
                if whole.is_empty() || !whole.bytes().all(|c|c.is_ascii_digit())
                    || !fraction.bytes().all(|c|c.is_ascii_digit()) || digits.ends_with('.') { return None; }
                let whole = whole.trim_start_matches('0');
                let fraction = fraction.trim_end_matches('0');
                let sign = if value.starts_with('-') && !(whole.is_empty() && fraction.is_empty()) { "-" } else { "" };
                let whole = if whole.is_empty() { "0" } else { whole };
                Some(if fraction.is_empty() { format!("{sign}{whole}") } else { format!("{sign}{whole}.{fraction}") })
            }
            Self::Checkbox => {
                if value.eq_ignore_ascii_case("true") { Some("true".into()) }
                else if value.eq_ignore_ascii_case("false") { Some("false".into()) } else { None }
            }
            Self::Date => {
                let b = value.as_bytes();
                if b.len() != 10 || b[4] != b'-' || b[7] != b'-' ||
                    b.iter().enumerate().any(|(i,c)|i!=4 && i!=7 && !c.is_ascii_digit()) { return None; }
                let year: u16 = value[..4].parse().ok()?;
                let month: u8 = value[5..7].parse().ok()?;
                let day: u8 = value[8..].parse().ok()?;
                let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                let max = match month { 2 if leap => 29, 2 => 28, 4|6|9|11 => 30, 1|3|5|7|8|10|12 => 31, _ => return None };
                (year != 0 && day >= 1 && day <= max).then(||value.into())
            }
        }
    }
}

pub(super) fn lookup(db:&Connection,key:&str) -> rusqlite::Result<Option<PropertyType>> {
    db.query_row("SELECT kind FROM property_definitions WHERE key=?1",[key],
                 |r| PropertyType::decode(r.get_ref(0)?.as_str()?)).optional()
}

pub(super) fn validate_write(db:&Connection,key:&str,value:&str) -> Result<()> {
    if let Some(kind) = lookup(db,key)? && kind.normalized(value).is_none() {
        return Err(format!("「{key}」需要輸入{}。",kind.hint()).into());
    }
    Ok(())
}

impl Notebook {
    pub(crate) fn property_type(&self,key:&str) -> Result<Option<PropertyType>> {
        Ok(lookup(&self.connection,key_checked(key)?)?)
    }

    pub(crate) fn property_definitions(&self) -> Result<Vec<(String,PropertyType)>> {
        let mut stmt = self.connection.prepare("SELECT key,kind FROM property_definitions ORDER BY key")?;
        Ok(stmt.query_map([],|r| Ok((r.get(0)?,PropertyType::decode(r.get_ref(1)?.as_str()?)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn compare_set_property_type(&self,key:&str,expected:Option<PropertyType>,next:Option<PropertyType>) -> Result<()> {
        let key = key_checked(key)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        if lookup(&tx,key)? != expected { return Err("其他視窗已更改這項屬性定義，請重新開啟。".into()); }
        if next != expected && tx.prepare("SELECT 1 FROM collection_field_rules WHERE key=?1")?.exists([key])? {
            return Err("仍有集合使用這個欄位的規則。請先移除規則，再更改或移除欄位型別。".into());
        }
        if let Some(kind) = next {
            if expected.is_none() && tx.query_row("SELECT count(*) FROM property_definitions",[],|r|r.get::<_,i64>(0))? >= 256 {
                return Err("工作區最多可設定 256 項屬性型別。".into());
            }
            tx.execute("INSERT INTO property_definitions(key,kind) VALUES(?1,?2)
                        ON CONFLICT(key) DO UPDATE SET kind=excluded.kind",params![key,kind.code()])?;
        } else { tx.execute("DELETE FROM property_definitions WHERE key=?1",[key])?; }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::collection_views::{CollectionLayout, PropertyFilter};

    #[test]
    fn typed_values_have_exact_decimal_and_calendar_semantics() {
        let number = PropertyType::Number;
        assert_eq!(number.normalized("-000.000"),number.normalized("+0"));
        assert_eq!(number.normalized("00012.0500").as_deref(),Some("12.05"));
        assert_ne!(number.normalized("9007199254740993"),number.normalized("9007199254740992"));
        for value in ["",".","-","1..2","NaN","1e3"] { assert!(number.normalized(value).is_none()); }
        for value in ["2023-02-29","1900-02-29","2000-13-02","2026-04-31","0000-01-01"] {
            assert!(PropertyType::Date.normalized(value).is_none());
        }
        assert!(PropertyType::Date.normalized("2000-02-29").is_some());
        assert_eq!(PropertyType::Checkbox.normalized(" TRUE ").as_deref(),Some("true"));
        assert!(PropertyType::Checkbox.normalized("yes").is_none());
    }

    #[test]
    fn definitions_keep_raw_text_and_saved_filters_keep_original_type() -> Result<()> {
        let mut db = Notebook::open(std::path::Path::new(":memory:"),false)?;
        let a = db.add(None,"甲","")?;
        let b = db.add(None,"乙","")?;
        let c = db.create_collection("專案")?;
        for id in [a,b] { db.add_to_collection(c,id,"處理中")?; }
        db.compare_set_property(a,"工時",None,Some("舊值"))?;
        db.compare_set_property_type("工時",None,Some(PropertyType::Number))?;
        assert!(db.compare_set_property_type("工時",None,Some(PropertyType::Date)).is_err());
        assert_eq!(db.properties(a)?[0].1,"舊值");
        assert!(db.compare_set_property(a,"工時",Some("舊值"),Some("其他")).is_err());
        db.compare_set_property(b,"工時",None,Some("002.500"))?;
        assert_eq!(db.properties(b)?[0].1,"002.500");
        let filter = PropertyFilter {key:"工時".into(),kind:PropertyType::Number,value:"+2.5".into()};
        let view = db.save_collection_view(c,"工時 2.5",CollectionLayout::Table,"","",Some(filter.clone()))?;
        assert_eq!(db.collection_views(c)?,[view.clone()]);
        db.compare_set_property_type("工時",Some(PropertyType::Number),Some(PropertyType::Date))?;
        assert!(db.save_collection_view(c,"舊定義",CollectionLayout::Table,"","",Some(filter)).is_err());
        let rows = db.filtered_collection_rows(c,Some(&view))?;
        assert_eq!(rows.iter().map(|(r,_)|r.id).collect::<Vec<_>>(),[b]);
        db.compare_set_property(a,"工時",Some("舊值"),None)?;
        assert!(db.properties(a)?.is_empty());
        assert_eq!(db.properties(b)?[0].1,"002.500");
        Ok(())
    }
}
