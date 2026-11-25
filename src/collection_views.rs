//! Named collection views select existing rows without changing membership or notes.
use super::{Notebook, Result, Row};
#[cfg(windows)]
pub(crate) use super::collections::{ChildDraft, CollectionTemplate, RelationMember, TemplateApply, TemplateChild, TemplateField, TemplateSelection, validate_template_body};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use super::typed_properties::{PropertyType, lookup};
use serde_json::{Value as Json, json};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use super::properties::{key_checked, write_value};
use super::formula_service::DerivedRow;
use crate::formula::{CompiledFormula, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PropertyFilter {
    pub key: String,
    pub kind: PropertyType,
    pub value: String,
}

pub(super) const SCHEMA: &str = "
CREATE TABLE collection_views (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
 name TEXT NOT NULL COLLATE NOCASE,
 layout TEXT NOT NULL CHECK(layout IN ('board','table')),
 status TEXT NOT NULL, keyword TEXT NOT NULL, UNIQUE(collection_id,name)
);";

pub(super) const PRESENTATION_SCHEMA: &str = "
ALTER TABLE collection_views ADD COLUMN calendar_key TEXT;
ALTER TABLE collection_views ADD COLUMN group_key TEXT;
ALTER TABLE collection_views ADD COLUMN sort_key TEXT;
ALTER TABLE collection_views ADD COLUMN sort_kind TEXT;
ALTER TABLE collection_views ADD COLUMN sort_desc INTEGER NOT NULL DEFAULT 0 CHECK(sort_desc IN (0,1));
ALTER TABLE collection_views ADD COLUMN calendar_month INTEGER;";

pub(super) const SCHEDULE_SCHEMA: &str = "
ALTER TABLE collection_views ADD COLUMN calendar_week_day TEXT;
ALTER TABLE collection_views ADD COLUMN calendar_from TEXT;
ALTER TABLE collection_views ADD COLUMN calendar_until TEXT;
ALTER TABLE collection_views ADD COLUMN list_mode INTEGER NOT NULL DEFAULT 0 CHECK(list_mode IN (0,1));";

pub(super) const ADVANCED_SCHEMA: &str = "
ALTER TABLE collection_views ADD COLUMN calendar_end_key TEXT;
ALTER TABLE collection_views ADD COLUMN rules_json TEXT;
ALTER TABLE collection_views ADD COLUMN settings_revision INTEGER NOT NULL DEFAULT 0;";

pub(super) const TIMELINE_SCHEMA: &str = "
ALTER TABLE collection_views ADD COLUMN timeline_mode INTEGER NOT NULL DEFAULT 0 CHECK(timeline_mode IN (0,1));";

pub(crate) const MAX_FILTERS: usize = 16;
pub(crate) const MAX_FILTER_GROUP_DEPTH: usize = 3;
pub(crate) const MAX_FILTER_GROUPS: usize = 16;
const MAX_SORTS: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum FilterMode {
    #[default]
    All,
    Any,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilterOperator {
    Equal, NotEqual, Less, LessOrEqual, Greater, GreaterOrEqual, IsEmpty, IsNotEmpty,
    Contains, NotContains, StartsWith, EndsWith,
    Today, Yesterday, Tomorrow, PreviousDays, NextDays,
    ThisWeek, PreviousWeek, NextWeek, ThisMonth, PreviousMonth, NextMonth,
}

impl FilterOperator {
    pub const OPTIONS: [(Self, &'static str); 23] = [
        (Self::Equal, "等於"), (Self::NotEqual, "不等於"),
        (Self::Less, "小於／早於"), (Self::LessOrEqual, "小於等於／當日或更早"),
        (Self::Greater, "大於／晚於"), (Self::GreaterOrEqual, "大於等於／當日或更晚"),
        (Self::IsEmpty, "空白"), (Self::IsNotEmpty, "非空白"),
        (Self::Contains, "包含"), (Self::NotContains, "不包含"),
        (Self::StartsWith, "開頭為"), (Self::EndsWith, "結尾為"),
        (Self::Today, "今天"), (Self::Yesterday, "昨天"), (Self::Tomorrow, "明天"),
        (Self::PreviousDays, "前 N 日（不含今天）"), (Self::NextDays, "後 N 日（不含今天）"),
        (Self::ThisWeek, "本週"), (Self::PreviousWeek, "上週"), (Self::NextWeek, "下週"),
        (Self::ThisMonth, "本月"), (Self::PreviousMonth, "上個月"), (Self::NextMonth, "下個月"),
    ];

    pub fn label(self) -> &'static str {
        Self::OPTIONS[self as usize].1
    }

    pub fn needs_value(self) -> bool {
        if self.relative() { return self.day_window(); }
        !matches!(self, Self::IsEmpty | Self::IsNotEmpty)
    }

    pub fn day_window(self) -> bool { matches!(self, Self::PreviousDays | Self::NextDays) }

    fn relative(self) -> bool {
        self.day_window() || matches!(self,
            Self::Today | Self::Yesterday | Self::Tomorrow |
            Self::ThisWeek | Self::PreviousWeek | Self::NextWeek |
            Self::ThisMonth | Self::PreviousMonth | Self::NextMonth)
    }

    fn matches_relative(self, value: CalendarDay, today: CalendarDay, raw_count: Option<&str>) -> bool {
        let delta = || value.ordinal() - today.ordinal();
        // Day 1 of the proleptic Gregorian calendar was a Monday.
        let from_monday = || {
            let today = today.ordinal();
            value.ordinal() - (today - (today - 1).rem_euclid(7))
        };
        let months = || value.month().serial() - today.month().serial();
        match self {
            Self::Today => delta() == 0,
            Self::Yesterday => delta() == -1,
            Self::Tomorrow => delta() == 1,
            Self::PreviousDays => raw_count.and_then(relative_days).is_some_and(|days| (-days..=-1).contains(&delta())),
            Self::NextDays => raw_count.and_then(relative_days).is_some_and(|days| (1..=days).contains(&delta())),
            Self::ThisWeek => (0..7).contains(&from_monday()),
            Self::PreviousWeek => (-7..0).contains(&from_monday()),
            Self::NextWeek => (7..14).contains(&from_monday()),
            Self::ThisMonth => months() == 0,
            Self::PreviousMonth => months() == -1,
            Self::NextMonth => months() == 1,
            _ => false,
        }
    }

    fn matches_order(self, order: Ordering) -> bool {
        match self {
            Self::Equal => order.is_eq(),
            Self::NotEqual => !order.is_eq(),
            Self::Less => order.is_lt(),
            Self::LessOrEqual => !order.is_gt(),
            Self::Greater => order.is_gt(),
            Self::GreaterOrEqual => !order.is_lt(),
            _ => false,
        }
    }
}

fn relative_days(raw: &str) -> Option<i32> {
    let raw = raw.trim();
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
    raw.parse::<i32>().ok().filter(|days| (1..=365).contains(days))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilterKind {
    Typed(PropertyType), Text, Title, Status,
    Formula(Option<PropertyType>), Relation, RollupCount, RollupSum,
}

impl FilterKind {
    fn code(self) -> &'static str {
        match self {
            Self::Typed(kind) => kind.code(),
            Self::Text => "text",
            Self::Title => "title",
            Self::Status => "status",
            Self::Formula(Some(PropertyType::Number)) => "formula-number",
            Self::Formula(Some(PropertyType::Checkbox)) => "formula-checkbox",
            Self::Formula(Some(PropertyType::Date)) => "formula-date",
            Self::Formula(None) => "formula-text",
            Self::Relation => "relation",
            Self::RollupCount => "rollup-count",
            Self::RollupSum => "rollup-sum",
        }
    }

    fn decode(value: &str) -> rusqlite::Result<Self> {
        match value {
            "text" => Ok(Self::Text),
            "title" => Ok(Self::Title),
            "status" => Ok(Self::Status),
            "formula-number" => Ok(Self::Formula(Some(PropertyType::Number))),
            "formula-checkbox" => Ok(Self::Formula(Some(PropertyType::Checkbox))),
            "formula-date" => Ok(Self::Formula(Some(PropertyType::Date))),
            "formula-text" => Ok(Self::Formula(None)),
            "relation" => Ok(Self::Relation),
            "rollup-count" => Ok(Self::RollupCount),
            "rollup-sum" => Ok(Self::RollupSum),
            _ => Ok(Self::Typed(PropertyType::decode(value)?)),
        }
    }

    pub fn checkbox(self) -> bool {
        matches!(self, Self::Typed(PropertyType::Checkbox) | Self::Formula(Some(PropertyType::Checkbox)))
    }

    pub fn allows(self, op: FilterOperator) -> bool {
        use FilterOperator::*;
        if op.relative() { return matches!(self, Self::Typed(PropertyType::Date) | Self::Formula(Some(PropertyType::Date))); }
        match self {
            Self::Relation => matches!(op, IsEmpty | IsNotEmpty | Contains | NotContains),
            kind if kind.checkbox() => matches!(op, Equal | NotEqual | IsEmpty | IsNotEmpty),
            Self::Typed(_) | Self::Formula(Some(_)) | Self::RollupCount | Self::RollupSum =>
                !matches!(op, Contains | NotContains | StartsWith | EndsWith),
            _ => matches!(op, Equal | NotEqual | IsEmpty | IsNotEmpty | Contains | NotContains | StartsWith | EndsWith),
        }
    }

    fn normalized(self, raw: &str) -> Option<String> {
        match self {
            Self::Typed(kind) => kind.normalized(raw),
            Self::Formula(Some(PropertyType::Number)) | Self::RollupCount | Self::RollupSum =>
                raw.trim().parse::<f64>().ok().filter(|number| number.is_finite()).map(|number| number.to_string()),
            Self::Formula(Some(kind)) => kind.normalized(raw),
            Self::Relation => raw.trim().parse::<i64>().ok().filter(|id| *id > 0).map(|id| id.to_string()),
            _ => Some(raw.trim().to_lowercase()),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Typed(kind) => kind.hint(),
            Self::Formula(Some(PropertyType::Number)) | Self::RollupCount | Self::RollupSum => "有限數字",
            Self::Formula(Some(kind)) => kind.hint(),
            Self::Relation => "目標筆記的固定 ID",
            _ => "文字（不分大小寫）",
        }
    }

    fn property(self) -> bool { matches!(self, Self::Typed(_) | Self::Text) }
    fn derived(self) -> bool {
        matches!(self, Self::Formula(_) | Self::Relation | Self::RollupCount | Self::RollupSum)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FilterRule {
    pub key: String,
    pub kind: FilterKind,
    pub operator: FilterOperator,
    pub value: String,
    pub argument: Option<String>,
}

impl FilterRule {
    pub fn label(&self) -> String {
        match self.kind {
            FilterKind::Title => "筆記標題".into(),
            FilterKind::Status => "集合狀態".into(),
            FilterKind::Formula(_) => format!("公式「{}」", self.key),
            FilterKind::Relation => format!("關聯「{}」", self.key),
            FilterKind::RollupCount => format!("「{}」的彙總筆數", self.key),
            FilterKind::RollupSum => format!("「{}」的「{}」加總", self.key, self.argument.as_deref().unwrap_or_default()),
            _ => self.key.clone(),
        }
    }

    fn normalized_target(&self) -> Option<String> {
        if self.operator.day_window() {
            relative_days(&self.value).map(|days| days.to_string())
        } else {
            self.kind.normalized(&self.value)
        }
    }

    fn validate(&mut self, db: &Transaction<'_>, collection: i64) -> Result<()> {
        if self.kind.property() || self.kind.derived() {
            self.key = key_checked(&self.key)?.to_owned();
            if self.kind.property() {
                let expected = match self.kind { FilterKind::Typed(kind) => Some(kind), _ => None };
                if lookup(db, &self.key)? != expected {
                    return Err("篩選欄位型別已變更。請移除該規則並重新選取。".into());
                }
            }
            if matches!(self.kind, FilterKind::Formula(_)) {
                require_formula(db, collection, &self.key)?;
            }
        } else {
            self.key.clear();
        }
        if self.kind == FilterKind::RollupSum {
            let field = self.argument.as_deref().ok_or("請指定彙總加總的數值欄位。")?;
            self.argument = Some(key_checked(field)?.to_owned());
        } else if self.argument.is_some() {
            return Err("這個欄位不支援額外參數。".into());
        }
        if !self.kind.allows(self.operator) { return Err("這個欄位不支援選取的條件。".into()); }
        if self.operator.needs_value() {
            if self.value.len() > super::MAX_VALUE_BYTES || self.value.contains('\0') {
                return Err("篩選值過長或含有 NUL。".into());
            }
            if self.value.trim().is_empty() { return Err("請輸入篩選值，或改用空白條件。".into()); }
            let hint = if self.operator.day_window() { "1 到 365 的天數" } else { self.kind.hint() };
            let normalized = self.normalized_target().ok_or_else(|| format!("「{}」需要輸入{hint}。", self.label()))?;
            if self.operator.day_window() || matches!(self.kind, FilterKind::Typed(_) | FilterKind::Relation) {
                self.value = normalized;
            }
        } else {
            self.value.clear();
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FilterGroup {
    pub mode: FilterMode,
    pub filters: Vec<FilterRule>,
    pub groups: Vec<FilterGroup>,
}

impl FilterGroup {
    pub fn is_empty(&self) -> bool { self.filters.is_empty() && self.groups.is_empty() }

    pub fn counts(&self) -> (usize, usize) {
        let (leaves, groups) = Self::counts_in(&self.groups);
        (self.filters.len() + leaves, 1 + groups)
    }

    pub fn counts_in(groups: &[Self]) -> (usize, usize) {
        groups.iter().map(Self::counts).fold((0, 0), |(leaves, groups), (more_leaves, more_groups)| {
            (leaves + more_leaves, groups + more_groups)
        })
    }

    fn encode(&self) -> Json {
        json!([self.mode == FilterMode::Any, encode_filters(&self.filters),
            self.groups.iter().map(Self::encode).collect::<Vec<_>>()])
    }

    fn decode(raw: &Json, depth: usize, budget: &mut usize) -> rusqlite::Result<Self> {
        let invalid = rusqlite::Error::InvalidQuery;
        if depth > MAX_FILTER_GROUP_DEPTH || *budget == 0 { return Err(invalid); }
        *budget -= 1;
        let Some([mode, filters, groups]) = raw.as_array().map(Vec::as_slice) else { return Err(invalid) };
        let mode = if mode.as_bool().ok_or(rusqlite::Error::InvalidQuery)? { FilterMode::Any } else { FilterMode::All };
        let filters = decode_filters(filters)?;
        let groups = groups.as_array().ok_or(rusqlite::Error::InvalidQuery)?.iter()
            .map(|group| Self::decode(group, depth + 1, budget)).collect::<rusqlite::Result<Vec<_>>>()?;
        let group = Self { mode, filters, groups };
        if group.is_empty() { return Err(rusqlite::Error::InvalidQuery); }
        Ok(group)
    }
}

fn encode_filters(filters: &[FilterRule]) -> Vec<Json> {
    filters.iter().map(|rule| {
        let mut values = vec![json!(rule.key), json!(rule.kind.code()), json!(rule.operator as usize), json!(rule.value)];
        if let Some(argument) = &rule.argument { values.push(json!(argument)); }
        Json::Array(values)
    }).collect()
}

fn decode_filters(raw: &Json) -> rusqlite::Result<Vec<FilterRule>> {
    let invalid = || rusqlite::Error::InvalidQuery;
    let filters = raw.as_array().ok_or_else(invalid)?;
    if filters.len() > MAX_FILTERS { return Err(invalid()); }
    filters.iter().map(|raw| {
        let tuple = raw.as_array().ok_or_else(invalid)?;
        if !(4..=5).contains(&tuple.len()) { return Err(invalid()); }
        let (key, kind, index, value): (String, String, usize, String) =
            serde_json::from_value(Json::Array(tuple[..4].to_vec())).map_err(|_| invalid())?;
        let argument = tuple.get(4).map(|raw| raw.as_str().map(str::to_owned).ok_or_else(invalid)).transpose()?;
        let operator = FilterOperator::OPTIONS.get(index).ok_or_else(invalid)?.0;
        let kind = FilterKind::decode(&kind)?;
        let checked = |key: &str| key_checked(key).ok() == Some(key);
        let valid_key = if kind.property() || kind.derived() { checked(&key) } else { key.is_empty() };

        let valid_argument = if kind == FilterKind::RollupSum { argument.as_deref().is_some_and(checked) } else { argument.is_none() };
        let rule = FilterRule { key, kind, operator, value, argument };
        let valid_value = !operator.needs_value() || (!rule.value.trim().is_empty() && rule.normalized_target().is_some());
        if !valid_key || !valid_value || !valid_argument || !kind.allows(operator)
            || rule.value.len() > super::MAX_VALUE_BYTES || rule.value.contains('\0')
        {
            return Err(invalid());
        }
        Ok(rule)
    }).collect()
}

fn require_formula(db: &Connection, collection: i64, name: &str) -> Result<()> {
    let exists = db.prepare_cached("SELECT 1 FROM collection_formulas WHERE collection_id=?1 AND name=?2")?
        .exists(params![collection, name])?;
    if !exists { return Err(format!("找不到集合公式「{name}」。請重新選取。").into()); }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SortSource {
    Property(Option<PropertyType>),
    Formula(Option<PropertyType>),
    RollupCount,
    RollupSum(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SortRule {
    pub key: String,
    pub source: SortSource,
    pub descending: bool,
}

impl SortRule {
    pub fn same_column(&self, other: &Self) -> bool {
        self.key == other.key && match (&self.source, &other.source) {
            (SortSource::Property(_), SortSource::Property(_))
            | (SortSource::Formula(_), SortSource::Formula(_))
            | (SortSource::RollupCount, SortSource::RollupCount) => true,
            (SortSource::RollupSum(a), SortSource::RollupSum(b)) => a == b,
            _ => false,
        }
    }

    pub fn label(&self) -> String {
        match &self.source {
            SortSource::Property(_) => self.key.clone(),
            SortSource::Formula(_) => format!("公式「{}」", self.key),
            SortSource::RollupCount => format!("「{}」的彙總筆數", self.key),
            SortSource::RollupSum(field) => format!("「{}」的「{field}」加總", self.key),
        }
    }

    fn encode(&self) -> Json {
        let code = match &self.source {
            SortSource::Property(kind) => kind.map(PropertyType::code),
            SortSource::Formula(kind) => Some(FilterKind::Formula(*kind).code()),
            SortSource::RollupCount => Some(FilterKind::RollupCount.code()),
            SortSource::RollupSum(field) =>
                return json!([self.key, FilterKind::RollupSum.code(), self.descending, field]),
        };
        json!([self.key, code, self.descending])
    }

    fn decode(value: &Json) -> rusqlite::Result<Self> {
        let bad = || rusqlite::Error::InvalidQuery;
        let parts = value.as_array().ok_or_else(bad)?;
        let [key, code, descending, rest @ ..] = parts.as_slice() else { return Err(bad()); };
        let key = key.as_str().filter(|key| key_checked(key).is_ok_and(|canonical| canonical == *key)).ok_or_else(bad)?;
        let source = match (code.is_null(), rest) {
            (true, []) => SortSource::Property(None),
            (false, []) => match FilterKind::decode(code.as_str().ok_or_else(bad)?)? {
                FilterKind::Typed(kind) => SortSource::Property(Some(kind)),
                FilterKind::Formula(kind) => SortSource::Formula(kind),
                FilterKind::RollupCount => SortSource::RollupCount,
                _ => return Err(bad()),
            },
            (false, [field]) if code.as_str() == Some(FilterKind::RollupSum.code()) => {
                let field = field.as_str().filter(|key| key_checked(key).is_ok_and(|canonical| canonical == *key))
                    .ok_or_else(bad)?;
                SortSource::RollupSum(field.to_owned())
            }
            _ => return Err(bad()),
        };
        Ok(Self { key: key.to_owned(), source, descending: descending.as_bool().ok_or_else(bad)? })
    }

    fn validate(&mut self, tx: &Transaction<'_>, collection: i64) -> Result<()> {
        self.key = key_checked(&self.key)?.to_owned();
        match &mut self.source {
            SortSource::Property(kind) if lookup(tx, &self.key)? != *kind =>
                Err("排序欄位型別已變更。請移除該規則並重新選取。".into()),
            SortSource::Formula(_) => require_formula(tx, collection, &self.key),
            SortSource::RollupSum(field) => {
                *field = key_checked(field)?.to_owned();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn value_type(&self) -> Option<PropertyType> {
        match &self.source {
            SortSource::Property(kind) | SortSource::Formula(kind) => *kind,
            SortSource::RollupCount | SortSource::RollupSum(_) => Some(PropertyType::Number),
        }
    }

    fn sort_value(
        &self, note: i64, columns: &HashMap<&str, HashMap<i64, String>>,
        formulas: &CompiledFields, derived: Option<&DerivedRow<'_>>,
    ) -> std::result::Result<Option<String>, String> {
        if let SortSource::Property(kind) = self.source {
            let raw = columns[self.key.as_str()].get(&note);
            return Ok(match (kind, raw) {
                (Some(kind), Some(raw)) => kind.normalized(raw),
                (None, Some(raw)) if !raw.is_empty() => Some(raw.clone()),
                _ => None,
            });
        }
        let row = derived.ok_or("找不到排序欄位的來源資料列。")?;
        let value = match &self.source {
            SortSource::Formula(expected) => match (expected, row.formula(&self.key, compiled_formula(formulas, &self.key)?)?) {
                (Some(PropertyType::Number), Value::Num(value)) => computed_decimal(value)?,
                (Some(PropertyType::Checkbox), Value::Bool(value)) => value.to_string(),
                (Some(PropertyType::Date), Value::Date(value)) => value,
                (None, Value::Str(value)) if value.trim().is_empty() => return Ok(None),
                (None, Value::Str(value)) => value,
                _ => return Err("公式結果的型別與排序設定不符。".to_owned()),
            },
            SortSource::RollupCount => row.count(&self.key)?.to_string(),
            SortSource::RollupSum(field) => computed_decimal(row.sum(&self.key, field)?)?,
            SortSource::Property(_) => unreachable!("property handled before evaluating derived rows"),
        };
        Ok(Some(value))
    }
}

fn computed_decimal(value: f64) -> std::result::Result<String, String> {
    if !value.is_finite() { return Err("排序數值超出範圍。".to_owned()); }
    // Display uses decimal notation. Normalize negative zero for the exact
    // decimal comparator shared with declared numeric properties.
    Ok(if value == 0.0 { "0".into() } else { value.to_string() })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ViewRules {
    pub mode: FilterMode,
    pub filters: Vec<FilterRule>,
    pub groups: Vec<FilterGroup>,
    pub sorts: Vec<SortRule>,
}

impl ViewRules {
    pub fn first_checkbox(&self) -> Option<&FilterRule> {
        fn from<'a>(filters: &'a [FilterRule], groups: &'a [FilterGroup]) -> Option<&'a FilterRule> {
            filters.iter().find(|filter| filter.kind == FilterKind::Typed(PropertyType::Checkbox))
                .or_else(|| groups.iter().find_map(|group| from(&group.filters, &group.groups)))
        }
        from(&self.filters, &self.groups)
    }

    fn encode(&self) -> String {
        let sorts = self.sorts.iter().map(SortRule::encode).collect::<Vec<_>>();
        let mut root = vec![json!(self.mode == FilterMode::Any), json!(encode_filters(&self.filters)), json!(sorts)];
        // Preserve the original typed flat tuple when it can express the current filter.
        if !self.groups.is_empty() {
            root.push(json!(self.groups.iter().map(FilterGroup::encode).collect::<Vec<_>>()));
        }
        Json::Array(root).to_string()
    }

    fn decode(raw: &str) -> rusqlite::Result<Self> {
        let invalid = || rusqlite::Error::InvalidQuery;
        if raw.len() > 7 * (MAX_FILTERS * (super::MAX_VALUE_BYTES + 2 * super::MAX_KEY_BYTES) + 2 * MAX_SORTS * super::MAX_KEY_BYTES + 1024) {
            return Err(invalid());
        }
        let json: Json = serde_json::from_str(raw).map_err(|_| invalid())?;
        let root = json.as_array().ok_or_else(invalid)?;
        if !(3..=4).contains(&root.len()) { return Err(invalid()); }
        let mode = if root[0].as_bool().ok_or_else(invalid)? { FilterMode::Any } else { FilterMode::All };
        let filters = decode_filters(&root[1])?;
        let sorts = root[2].as_array().ok_or_else(invalid)?;
        if sorts.len() > MAX_SORTS { return Err(invalid()); }
        let mut budget = MAX_FILTER_GROUPS;
        let groups = if let Some(groups) = root.get(3) {
            groups.as_array().ok_or_else(invalid)?.iter()
                .map(|group| FilterGroup::decode(group, 1, &mut budget)).collect::<rusqlite::Result<Vec<_>>>()?
        } else { Vec::new() };
        let root = Self {
            mode, filters, groups,
            sorts: sorts.iter().map(SortRule::decode).collect::<rusqlite::Result<_>>()?,
        };
        if !root.counts_are_valid() { return Err(invalid()); }
        Ok(root)
    }

    fn counts_are_valid(&self) -> bool {
        let (leaves, groups) = FilterGroup::counts_in(&self.groups);
        self.filters.len() + leaves <= MAX_FILTERS && groups <= MAX_FILTER_GROUPS && self.sorts.len() <= MAX_SORTS
    }

    fn validate(&mut self, tx: &Transaction<'_>, collection: i64) -> Result<()> {
        if !self.counts_are_valid() {
            return Err("每個檢視最多可有 16 個篩選條件、16 個群組和 8 個排序欄。".into());
        }
        fn validate_group(group: &mut FilterGroup, depth: usize, tx: &Transaction<'_>, collection: i64) -> Result<()> {
            if depth > MAX_FILTER_GROUP_DEPTH { return Err("最多可在主條件下使用 3 層群組。".into()); }
            if group.is_empty() { return Err("篩選群組需要至少一個條件或子群組。".into()); }
            for rule in &mut group.filters { rule.validate(tx, collection)?; }
            for sub in &mut group.groups { validate_group(sub, depth + 1, tx, collection)?; }
            Ok(())
        }
        for rule in &mut self.filters { rule.validate(tx, collection)?; }
        for group in &mut self.groups { validate_group(group, 1, tx, collection)?; }
        for index in 0..self.sorts.len() {
            let (prior, next) = self.sorts.split_at_mut(index);
            let rule = &mut next[0];
            rule.validate(tx, collection)?;
            if prior.iter().any(|previous| previous.same_column(rule)) {
                return Err("同一個欄位只能排序一次。".into());
            }
        }
        Ok(())
    }
}

type CompiledFields = HashMap<String, std::result::Result<CompiledFormula, String>>;

fn compiled_formula<'a>(formulas: &'a CompiledFields, name: &str) -> std::result::Result<&'a CompiledFormula, String> {
    formulas.get(name).ok_or_else(|| format!("找不到集合公式「{name}」。"))?.as_ref().map_err(Clone::clone)
}


#[derive(Default)]
pub(crate) struct RuleProblems {
    pub entries: Vec<String>,
    pub omitted: usize,
}

impl RuleProblems {
    fn record(&mut self, note: i64, operation: &str, field: &str, error: &str) {
        if self.entries.len() < 64 {
            self.entries.push(format!("筆記 #{note}　{operation}　{field}　{error}"));
        } else {
            self.omitted += 1;
        }
    }
}

struct PreparedFilter<'a> {
    mode: FilterMode,
    filters: Vec<(&'a FilterRule, Option<String>)>,
    groups: Vec<PreparedFilter<'a>>,
}

impl<'a> PreparedFilter<'a> {
    fn new(mode: FilterMode, filters: &'a [FilterRule], groups: &'a [FilterGroup]) -> Result<Self> {
        let filters = filters.iter().map(|rule| {
            let target = if rule.operator.needs_value() {
                Some(rule.normalized_target().ok_or("儲存的篩選值格式有誤。")?)
            } else { None };
            Ok((rule, target))
        }).collect::<Result<Vec<_>>>()?;
        let groups = groups.iter().map(|group| Self::new(group.mode, &group.filters, &group.groups))
            .collect::<Result<_>>()?;
        Ok(Self { mode, filters, groups })
    }

    fn sources(&self, properties: &mut HashSet<&'a str>, formulas: &mut HashSet<&'a str>) -> bool {
        let mut needs_derived = false;
        for (rule, _) in &self.filters {
            if rule.kind.property() { properties.insert(&rule.key); }
            if matches!(rule.kind, FilterKind::Formula(_)) { formulas.insert(&rule.key); }
            needs_derived |= rule.kind.derived();
        }
        for group in &self.groups { needs_derived |= group.sources(properties, formulas); }
        needs_derived
    }

    fn needs_today(&self) -> bool {
        self.filters.iter().any(|(rule, _)| rule.operator.relative())
            || self.groups.iter().any(Self::needs_today)
    }

    fn matches(&self, row: &Row, status: &str, columns: &HashMap<&str, HashMap<i64, String>>,
               formulas: &CompiledFields, derived: Option<&DerivedRow<'_>>, today: Option<CalendarDay>, issue: &mut impl FnMut(&FilterRule, &str)) -> bool {
        if self.filters.is_empty() && self.groups.is_empty() { return true; }
        let want_all = self.mode == FilterMode::All;
        for (rule, target) in &self.filters {
            let needle = target.as_deref();
            let matched = if rule.kind.derived() {
                match derived_match(rule, needle, formulas, derived, today) {
                    Ok(matched) => matched,
                    Err(error) => {
                        issue(rule, &error);
                        false
                    }
                }
            } else {
                let raw = match rule.kind {
                    FilterKind::Title => Some(row.title.as_str()),
                    FilterKind::Status => Some(status),
                    _ => columns[rule.key.as_str()].get(&row.id).map(String::as_str),
                };
                text_match(rule, raw, needle, today)
            };
            if matched != want_all { return matched; }
        }
        for group in &self.groups {
            let matched = group.matches(row, status, columns, formulas, derived, today, issue);
            if matched != want_all { return matched; }
        }
        want_all
    }
}

fn text_match(rule: &FilterRule, raw: Option<&str>, target: Option<&str>, today: Option<CalendarDay>) -> bool {
    use FilterOperator::*;
    match rule.operator {
        op if op.relative() => raw.and_then(CalendarDay::parse).zip(today)
            .is_some_and(|(value, today)| op.matches_relative(value, today, target)),
        IsEmpty => raw.is_none_or(|raw| raw.trim().is_empty()),
        IsNotEmpty => raw.is_some_and(|raw| !raw.trim().is_empty()),
        op => raw.and_then(|raw| rule.kind.normalized(raw)).zip(target).is_some_and(|(value, target)| {
            match op {
                Contains => value.contains(target),
                NotContains => !value.contains(target),
                StartsWith => value.starts_with(target),
                EndsWith => value.ends_with(target),
                _ => {
                    let kind = match rule.kind {
                        FilterKind::Typed(kind) | FilterKind::Formula(Some(kind)) => Some(kind),
                        _ => None,
                    };
                    op.matches_order(compare_value(kind, &value, target))
                }
            }
        }),
    }
}

fn derived_match(
    rule: &FilterRule, target: Option<&str>, formulas: &CompiledFields, row: Option<&DerivedRow<'_>>, today: Option<CalendarDay>,
) -> std::result::Result<bool, String> {
    use FilterOperator::*;
    let row = row.ok_or("找不到衍生欄位的來源資料列。")?;
    let number_matches = |value: f64| -> std::result::Result<bool, String> {
        match rule.operator {
            IsEmpty => Ok(false), IsNotEmpty => Ok(true),
            op => {
                let number = target.and_then(|target| target.parse::<f64>().ok()).ok_or("篩選值必須是有限數字。")?;
                let order = value.partial_cmp(&number).ok_or("數值超出範圍。")?;
                Ok(op.matches_order(order))
            }
        }
    };
    match rule.kind {
        FilterKind::Formula(expected) => {
            let code = compiled_formula(formulas, &rule.key)?;
            match (expected, row.formula(&rule.key, code)?) {
                (Some(PropertyType::Number), Value::Num(value)) => number_matches(value),
                (Some(PropertyType::Checkbox), Value::Bool(value)) =>
                    Ok(text_match(rule, Some(if value { "true" } else { "false" }), target, today)),
                (Some(PropertyType::Date), Value::Date(value)) | (None, Value::Str(value)) =>
                    Ok(text_match(rule, Some(&value), target, today)),
                _ => Err(format!("公式結果的型別與規則不符，這條規則需要{}。", rule.kind.hint())),
            }
        }
        FilterKind::Relation => {
            if matches!(rule.operator, IsEmpty | IsNotEmpty) {
                let count = row.count(&rule.key)?;
                return Ok((count == 0) == (rule.operator == IsEmpty));
            }
            let id = target.and_then(|target| target.parse::<i64>().ok()).ok_or("找不到關聯規則的目標編號。")?;
            let present = row.relation_has(&rule.key, id)?;
            Ok(present == (rule.operator == Contains))
        }
        FilterKind::RollupCount => number_matches(row.count(&rule.key)? as f64),
        FilterKind::RollupSum => number_matches(row.sum(&rule.key, rule.argument.as_deref().ok_or("加總規則缺少數值欄位。")?)?),
        _ => Err("不是衍生欄位。".into()),
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct CalendarDay {
    year:u16,
    month:u16,
    day:u16,
}

impl CalendarDay {
    pub const LAST: Self = Self { year: 9999, month: 12, day: 31 };

    pub fn parse(raw:&str) -> Option<Self> {
        let raw = PropertyType::Date.normalized(raw)?;
        Some(Self {year:raw[..4].parse().ok()?,month:raw[5..7].parse().ok()?,day:raw[8..].parse().ok()?})
    }

    #[cfg(windows)]
    pub fn local_today() -> Result<Self> {
        let mut now = unsafe { std::mem::zeroed() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut now); }
        Self::parse(&format!("{:04}-{:02}-{:02}", now.wYear, now.wMonth, now.wDay))
            .ok_or_else(|| "無法取得今天的日期。".into())
    }

    #[cfg(not(windows))]
    fn local_today() -> Result<Self> {
        use std::time::{SystemTime, UNIX_EPOCH};
        // The app UI is Win32. Host-side regressions can inject a date; UTC is the host fallback.
        let mut days = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() / 86_400;
        let mut month = Month { year: 1970, month: 1 };
        while days >= month.days() as u64 {
            days -= month.days() as u64;
            let next = month.shifted(1);
            if next == month { return Err("無法取得今天的日期。".into()); }
            month = next;
        }
        Ok(Self { year: month.year, month: month.month, day: days as u16 + 1 })
    }

    fn ordinal(self) -> i32 {
        let prior_year = i32::from(self.year) - 1;
        let elapsed_months: i32 = (1..self.month).map(|month|
            (Month { year: self.year, month }).days() as i32
        ).sum();
        prior_year * 365 + prior_year / 4 - prior_year / 100 + prior_year / 400 + elapsed_months + i32::from(self.day)
    }

    pub fn iso(self) -> String { format!("{:04}-{:02}-{:02}",self.year,self.month,self.day) }

    pub fn month(self) -> Month { Month {year:self.year,month:self.month} }

    pub fn day(self) -> usize { usize::from(self.day) }

    pub fn shifted(mut self, delta:i8) -> Option<Self> {
        for _ in 0..delta.unsigned_abs() {
            let month = self.month();
            if delta > 0 {
                if self.day() < month.days() { self.day += 1; }
                else {
                    let next = month.shifted(1);
                    if next == month { return None; }
                    self = Self {year:next.year,month:next.month,day:1};
                }
            } else if self.day > 1 {
                self.day -= 1;
            } else {
                let prev = month.shifted(-1);
                if prev == month { return None; }
                self = Self {year:prev.year,month:prev.month,day:prev.days() as u16};
            }
        }
        Some(self)
    }

    pub fn monday(self) -> Self {
        let offset = (self.month().sunday_offset()+self.day()+5)%7;
        self.shifted(-(offset as i8)).unwrap_or(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScheduleAdjustment { Move, Start, End }

impl ScheduleAdjustment {
    pub(crate) fn apply(self, start: CalendarDay, end: Option<CalendarDay>, delta: i8) -> Option<(CalendarDay, Option<CalendarDay>)> {
        if delta == 0 { return Some((start, end)); }
        let (start, end) = match self {
            Self::Move => (start.shifted(delta)?, if let Some(end) = end { Some(end.shifted(delta)?) } else { None }),
            Self::Start => (start.shifted(delta)?, Some(end?)),
            Self::End => (start, Some(end.unwrap_or(start).shifted(delta)?)),
        };
        end.is_none_or(|end| end >= start).then_some((start, end))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Presentation {
    pub calendar_key: Option<String>,
    pub calendar_end_key: Option<String>,
    pub group_key: Option<String>,
    pub sort_key: Option<String>,
    pub sort_kind: Option<PropertyType>,
    pub sort_desc: bool,
    pub calendar_month: Option<Month>,
    pub calendar_week_day: Option<CalendarDay>,
    pub calendar_from: Option<CalendarDay>,
    pub calendar_until: Option<CalendarDay>,
    pub list_mode: bool,
    pub timeline_mode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Month {
    pub year: u16,
    pub month: u16,
}

impl Month {
    pub fn parse(raw: &str) -> Option<Self> {
        CalendarDay::parse(&format!("{}-01", raw.trim())).map(CalendarDay::month)
    }

    pub fn iso(self) -> String { format!("{:04}-{:02}", self.year, self.month) }

    pub fn bounds(self) -> (CalendarDay, CalendarDay) {
        let start = CalendarDay { year: self.year, month: self.month, day: 1 };
        let end = CalendarDay { day: self.days() as u16, ..start };
        (start, end)
    }

    fn serial(self) -> i32 { i32::from(self.year)*12+i32::from(self.month)-1 }

    fn from_serial(value:i32) -> Option<Self> {
        (12..120000).contains(&value).then_some(Self {year:(value/12) as u16,month:(value%12+1) as u16})
    }

    pub fn shifted(self, delta: i32) -> Self {
        let offset = (i32::from(self.year)-1)*12+i32::from(self.month)-1;
        let target = offset.saturating_add(delta).clamp(0,9999*12-1);
        Self {year:(target/12+1) as u16,month:(target%12+1) as u16}
    }

    pub fn days(self) -> usize {
        match self.month {
            2 if self.year % 4 == 0 && (self.year % 100 != 0 || self.year % 400 == 0) => 29,
            2 => 28, 4|6|9|11 => 30, _ => 31,
        }
    }

    pub fn sunday_offset(self) -> usize {
        let prior = usize::from(self.year)-1;
        let leap_days = prior/4-prior/100+prior/400;
        let elapsed: usize = (1..self.month).map(|month| Self {year:self.year,month}.days()).sum();
        // Proleptic Gregorian 0001-01-01 is Monday.
        (1+prior*365+leap_days+elapsed)%7
    }
}

pub(crate) struct ScheduledRow {
    pub row: Row,
    pub status: String,
    pub start: CalendarDay,
    pub end: CalendarDay,
    pub visible_start: CalendarDay,
    pub visible_end: CalendarDay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CollectionLayout { Board, Table }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollectionView {
    pub id: i64,
    pub name: String,
    pub layout: CollectionLayout,
    pub status: String,
    pub keyword: String,
    pub filter: Option<PropertyFilter>,
    pub presentation: Presentation,
    rules: Option<ViewRules>,
    settings_revision: i64,
}

impl CollectionView {
    pub fn rules(&self) -> ViewRules {
        self.rules.clone().unwrap_or_else(|| ViewRules {
            mode: FilterMode::All,
            filters: self.filter.as_ref().map(|rule| FilterRule {
                key: rule.key.clone(), kind: FilterKind::Typed(rule.kind), operator: FilterOperator::Equal, value: rule.value.clone(), argument: None,
            }).into_iter().collect(),
            groups: Vec::new(),
            sorts: self.sort_rules(),
        })
    }

    pub fn sort_rules(&self) -> Vec<SortRule> {
        if let Some(rules) = &self.rules { return rules.sorts.clone(); }
        self.presentation.sort_key.as_ref().map(|key| SortRule {
            key: key.clone(), source: SortSource::Property(self.presentation.sort_kind), descending: self.presentation.sort_desc,
        }).into_iter().collect()
    }
}

pub(crate) struct FieldEdit<'a> {
    pub key: &'a str,
    pub kind: PropertyType,
    pub previous: Option<&'a str>,
    pub value: Option<&'a str>,
}

fn field(value: &str) -> Result<&str> {
    if value.len() > 256 || value.chars().any(char::is_control) {
        return Err("檢視名稱和篩選條件各限 256 位元組，不能含控制字元。".into());
    }
    Ok(value.trim())
}

impl Notebook {
    pub(crate) fn create_collection_view(&self, collection: i64, name: &str, layout: CollectionLayout,
                                         status: &str, keyword: &str) -> Result<CollectionView> {
        self.save_collection_view(collection,name,layout,status,keyword,None)
    }

    pub(crate) fn save_collection_view(&self, collection:i64, name:&str, layout:CollectionLayout,
                                       status:&str, keyword:&str, filter:Option<PropertyFilter>) -> Result<CollectionView> {
        self.save_presented_collection_view(collection,name,layout,status,keyword,filter,Presentation::default())
    }

    pub(crate) fn save_presented_collection_view(
        &self, collection:i64, name:&str, layout:CollectionLayout, status:&str, keyword:&str,
        filter:Option<PropertyFilter>, mut presentation:Presentation,
    ) -> Result<CollectionView> {
        let (name, status, keyword) = (collection_view_name(name)?, field(status)?, field(keyword)?);
        let kind = match layout { CollectionLayout::Board => "board", CollectionLayout::Table => "table" };
        let tx = Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        for key in [&mut presentation.calendar_key, &mut presentation.calendar_end_key, &mut presentation.group_key, &mut presentation.sort_key] {
            if let Some(raw) = key.as_mut() { *raw = key_checked(raw)?.to_owned(); }
        }
        if let Some(key) = &presentation.calendar_key {
            if layout != CollectionLayout::Table || lookup(&tx,key)? != Some(PropertyType::Date) {
                return Err("日期檢視需要已宣告的日期屬性。".into());
            }
        }
        validate_end_key(&tx, presentation.calendar_key.as_deref(), presentation.calendar_end_key.as_deref())?;
        if let Some(month) = presentation.calendar_month {
            if presentation.calendar_key.is_none() || !(1..=9999).contains(&month.year) || !(1..=12).contains(&month.month) {
                return Err("無效的月曆月份。".into());
            }
        }
        validate_calendar_range(presentation.calendar_from,presentation.calendar_until)?;
        if presentation.calendar_key.is_none()
            && (presentation.calendar_week_day.is_some() || presentation.calendar_from.is_some() || presentation.calendar_until.is_some())
        { return Err("日期條件僅適用於月曆。".into()); }
        if presentation.list_mode && (layout != CollectionLayout::Table || presentation.calendar_key.is_some()) {
            return Err("無效的清單檢視。".into());
        }
        if presentation.timeline_mode && (presentation.calendar_key.is_none() || presentation.calendar_month.is_none()
            || presentation.calendar_week_day.is_some() || presentation.list_mode)
        {
            return Err("時間軸需要日期欄和指定月份。".into());
        }
        if let Some(week) = &mut presentation.calendar_week_day { *week = week.monday(); }
        presentation.sort_kind = if let Some(key) = &presentation.sort_key {
            lookup(&tx,key)?
        } else {
            presentation.sort_desc = false;
            None
        };
        if let Some(f) = &filter {
            if lookup(&tx,&f.key)? != Some(f.kind) { return Err("屬性型別已更改，請重新選取。".into()); }
            if f.value.len() > super::MAX_VALUE_BYTES || f.kind.normalized(&f.value).is_none() {
                return Err(format!("「{}」需要輸入{}。",f.key,f.kind.hint()).into());
            }
        }
        let changed = tx.execute(
            "INSERT INTO collection_views(collection_id,name,layout,status,keyword,filter_key,filter_kind,filter_value,
                    calendar_key,group_key,sort_key,sort_kind,sort_desc,calendar_month,
                    calendar_week_day,calendar_from,calendar_until,list_mode,calendar_end_key,timeline_mode)
             SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20 WHERE (SELECT count(*) FROM collection_views WHERE collection_id=?1)<64",
            params![collection,name,kind,status,keyword,filter.as_ref().map(|f|&f.key),
                    filter.as_ref().map(|f|f.kind.code()),filter.as_ref().map(|f|&f.value),
                    &presentation.calendar_key,&presentation.group_key,&presentation.sort_key,
                    presentation.sort_kind.map(PropertyType::code),presentation.sort_desc,presentation.calendar_month.map(Month::serial),
                    presentation.calendar_week_day.map(CalendarDay::iso),presentation.calendar_from.map(CalendarDay::iso),
                    presentation.calendar_until.map(CalendarDay::iso),presentation.list_mode,&presentation.calendar_end_key,presentation.timeline_mode],
        )?;
        if changed == 0 { return Err("每個集合最多可儲存 64 個檢視。".into()); }
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(CollectionView { id, name: name.into(), layout,
                            status: status.into(), keyword: keyword.into(), filter, presentation, rules:None, settings_revision:0 })
    }

    pub(crate) fn collection_views(&self, collection: i64) -> Result<Vec<CollectionView>> {
        collection_view_rows(&self.connection, collection, None)
    }

    pub(crate) fn rename_collection_view(&self, collection: i64, view: &CollectionView, name: &str) -> Result<()> {
        let name = collection_view_name(name)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        require_current_view(&tx, collection, view)?;
        if name != view.name {
            require_available_name(&tx, collection, name, Some(view.id))?;
            tx.execute(
                "UPDATE collection_views SET name=?3, settings_revision=settings_revision+1
                 WHERE collection_id=?1 AND id=?2",
                params![collection, view.id, name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn duplicate_collection_view(&self, collection: i64, source: &CollectionView, name: &str) -> Result<i64> {
        self.copy_collection_view_to(collection, source, collection, name)
    }

    pub(crate) fn copy_collection_view_to(
        &self, origin: i64, source: &CollectionView, destination: i64, name: &str,
    ) -> Result<i64> {
        let name = collection_view_name(name)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        require_current_view(&tx, origin, source)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE id=?1)", [destination], |row| row.get(0),
        )?;
        if !exists { return Err("找不到目標集合。".into()); }
        if destination != origin {
            for rule in source.sort_rules() {
                if matches!(rule.source, SortSource::Formula(_)) {
                    require_formula(&tx, destination, &rule.key)?;
                }
            }
        }
        require_available_name(&tx, destination, name, None)?;
        let changed = tx.execute(
            "INSERT INTO collection_views(collection_id,name,layout,status,keyword,filter_key,filter_kind,filter_value,
                     calendar_key,group_key,sort_key,sort_kind,sort_desc,calendar_month,calendar_week_day,calendar_from,
                     calendar_until,list_mode,calendar_end_key,rules_json,timeline_mode)
             SELECT ?3,?4,layout,status,keyword,filter_key,filter_kind,filter_value,
                    calendar_key,group_key,sort_key,sort_kind,sort_desc,calendar_month,calendar_week_day,calendar_from,
                    calendar_until,list_mode,calendar_end_key,rules_json,timeline_mode
               FROM collection_views WHERE collection_id=?1 AND id=?2
                AND (SELECT count(*) FROM collection_views WHERE collection_id=?3)<64",
            params![origin, source.id, destination, name],
        )?;
        if changed != 1 { return Err("每個集合最多可儲存 64 個檢視。".into()); }
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    pub(crate) fn edit_collection_field(
        &self, collection: i64, note_id: i64, status: &str, edit: FieldEdit<'_>,
    ) -> Result<()> {
        let key = key_checked(edit.key)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let member: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM collection_notes c JOIN notes n ON n.id=c.note_id
                WHERE c.collection_id=?1 AND c.note_id=?2 AND c.status=?3 AND n.trash_root IS NULL)",
            params![collection, note_id, status], |row| row.get(0),
        )?;
        if !member || lookup(&tx, key)? != Some(edit.kind) {
            return Err("資料列或欄位型別已變更，請重新整理。".into());
        }
        let previous: Option<String> = tx.query_row(
            "SELECT value FROM note_properties WHERE note_id=?1 AND key=?2",
            params![note_id, key], |row| row.get(0),
        ).optional()?;
        if previous.as_deref() != edit.previous {
            return Err("欄位值已由其他視窗變更，請重新開啟。".into());
        }
        if edit.value == edit.previous { return Ok(()); }
        if let Some(value) = edit.value
            && (value.len() > super::MAX_VALUE_BYTES || edit.kind.normalized(value).is_none())
        {
            return Err(format!("「{key}」需要輸入{}。", edit.kind.hint()).into());
        }
        if edit.kind == PropertyType::Date {
            validate_collection_field_dates(&tx, note_id, [(key, edit.value.and_then(CalendarDay::parse)), ("", None)])?;
        }
        if let Some(value) = edit.value {
            write_value(&tx, note_id, key, value, previous.is_none())?;
        } else {
            super::properties::delete_value(&tx, note_id, key)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn set_collection_basic_filter(
        &self, collection: i64, view: &CollectionView, status: &str, keyword: &str,
    ) -> Result<()> {
        let (status, keyword) = (field(status)?, field(keyword)?);
        let changed = self.connection.execute(
            "UPDATE collection_views SET status=?4,keyword=?5,settings_revision=settings_revision+1
             WHERE collection_id=?1 AND id=?2 AND settings_revision=?3",
            params![collection, view.id, view.settings_revision, status, keyword],
        )?;
        if changed != 1 { return Err("檢視設定已變更，請重新開啟。".into()); }
        Ok(())
    }

    pub(crate) fn set_collection_view_rules(&self, collection: i64, view: &CollectionView, mut rules: ViewRules) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        rules.validate(&tx, collection)?;
        let changed = tx.execute(
            "UPDATE collection_views SET rules_json=?4,settings_revision=settings_revision+1
             WHERE collection_id=?1 AND id=?2 AND settings_revision=?3",
            params![collection, view.id, view.settings_revision, rules.encode()],
        )?;
        if changed != 1 { return Err("檢視設定已變更，請重新開啟。".into()); }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn set_collection_end_key(&self, collection: i64, view: &CollectionView, end: Option<&str>) -> Result<()> {
        let end = end.map(key_checked).transpose()?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let start = view.presentation.calendar_key.as_deref().ok_or("請先開啟已儲存的月曆或週曆。")?;
        validate_end_key(&tx, Some(start), end)?;
        let changed = tx.execute(
            "UPDATE collection_views SET calendar_end_key=?4,settings_revision=settings_revision+1
             WHERE collection_id=?1 AND id=?2 AND settings_revision=?3 AND calendar_key=?5",
            params![collection, view.id, view.settings_revision, end, start],
        )?;
        if changed != 1 { return Err("檢視設定已變更，請重新開啟。".into()); }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn set_collection_row_dates(
        &self, collection: i64, view: &CollectionView, note_id: i64,
        expected: [Option<&str>; 2], values: [Option<CalendarDay>; 2],
    ) -> Result<()> {
        let [start, end] = values;
        validate_calendar_range(start, end)?;
        let start_key = view.presentation.calendar_key.as_deref().ok_or("目前的檢視未設定起始日期欄。")?;
        let end_key = view.presentation.calendar_end_key.as_deref();
        if end.is_some() && (start.is_none() || end_key.is_none()) {
            return Err("輸入結束日期前請先輸入起始日期，並為檢視指定結束日期欄。".into());
        }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        require_current_view(&tx, collection, view)?;
        let editable: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM collection_notes c JOIN notes n ON n.id=c.note_id
             WHERE c.collection_id=?1 AND c.note_id=?2 AND n.trash_root IS NULL)",
            params![collection, note_id], |row| row.get(0),
        )?;
        if !editable || lookup(&tx, start_key)? != Some(PropertyType::Date) {
            return Err("資料列或日期欄位已變更，請重新整理。".into());
        }
        validate_end_key(&tx, Some(start_key), end_key)?;
        let keys = [Some(start_key), end_key];
        let mut updates = [("", None); 2];
        for (index, key) in keys.into_iter().enumerate() {
            let Some(key) = key else { continue };
            let previous: Option<String> = tx.query_row(
                "SELECT value FROM note_properties WHERE note_id=?1 AND key=?2",
                params![note_id, key], |row| row.get(0),
            ).optional()?;
            if previous.as_deref() != expected[index] {
                return Err("起訖日期已由其他視窗變更，請重新開啟。".into());
            }
            let unchanged = match (previous.as_deref(), values[index]) {
                (None, None) => true,
                (Some(raw), Some(next)) => CalendarDay::parse(raw) == Some(next),
                _ => false,
            };
            if !unchanged { updates[index] = (key, values[index]); }
        }
        validate_collection_field_dates(&tx, note_id, updates)?;
        for (key, value) in updates {
            if !key.is_empty() && value.is_none() {
                super::properties::delete_value(&tx, note_id, key)?;
            }
        }
        for (index, (key, value)) in updates.into_iter().enumerate() {
            if let Some(date) = value {
                write_value(&tx, note_id, key, &date.iso(), expected[index].is_none())?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn filtered_collection_rows(&self, collection: i64, view: Option<&CollectionView>) -> Result<Vec<(Row,String)>> {
        self.filter_collection_rows(collection, view, None, None)
    }

    pub(crate) fn collection_rule_problems(&self, collection: i64, view: &CollectionView) -> Result<RuleProblems> {
        let mut problems = RuleProblems::default();
        self.filter_collection_rows(collection, Some(view), Some(&mut problems), None)?;
        Ok(problems)
    }

    fn filter_collection_rows(
        &self, collection: i64, view: Option<&CollectionView>, mut issues: Option<&mut RuleProblems>,
        as_of: Option<CalendarDay>,
    ) -> Result<Vec<(Row,String)>> {
        let Some(view) = view else { return self.collection_rows(collection) };
        let rules = view.rules();
        let filters = PreparedFilter::new(rules.mode, &rules.filters, &rules.groups)?;
        let today = if filters.needs_today() {
            Some(match as_of { Some(day) => day, None => CalendarDay::local_today()? })
        } else { None };
        let mut keys = HashSet::new();
        let mut formula_names = HashSet::new();
        let mut needs_derived = filters.sources(&mut keys, &mut formula_names);
        for rule in &rules.sorts {
            match &rule.source {
                SortSource::Property(_) => { keys.insert(&rule.key); }
                SortSource::Formula(_) => { formula_names.insert(&rule.key); needs_derived = true; }
                _ => needs_derived = true,
            }
        }
        let snapshot = (needs_derived && self.connection.is_autocommit())
            .then(|| self.connection.unchecked_transaction()).transpose()?;
        let mut rows = self.collection_rows(collection)?;
        let needle = view.keyword.to_lowercase();
        rows.retain(|(row,status)| (view.status.is_empty() || status == &view.status)
            && (needle.is_empty() || row.title.to_lowercase().contains(&needle)));
        let columns = keys.into_iter().map(|key| Ok((key, self.collection_values(collection, key)?)))
            .collect::<Result<HashMap<_, _>>>()?;
        let (formulas, types) = if formula_names.is_empty() {
            (HashMap::new(), HashMap::new())
        } else {
            let formulas = self.collection_formulas(collection)?.into_iter().filter(|(name, _)| formula_names.contains(name.as_str()))
                .map(|(name, expression)| (name, CompiledFormula::new(&expression).map_err(|error| error.to_string()))).collect();
            (formulas, self.property_definitions()?.into_iter().collect())
        };
        let mut ordered = rows.into_iter().filter_map(|item| {
            let (row, status) = &item;
            let derived = needs_derived.then(|| DerivedRow::new(self, row.id, &types));
            let matches = filters.matches(row, status, &columns, &formulas, derived.as_ref(), today, &mut |rule, error| {
                if let Some(issues) = issues.as_deref_mut() { issues.record(row.id, "篩選", &rule.label(), error); }
            });
            if !matches { return None; }
            let values = rules.sorts.iter().map(|rule| match rule.sort_value(row.id, &columns, &formulas, derived.as_ref()) {
                Ok(value) => value,
                Err(error) => {
                    if let Some(issues) = issues.as_deref_mut() { issues.record(row.id, "排序", &rule.label(), &error); }
                    None
                }
            }).collect::<Vec<_>>();
            Some((item, values))
        }).collect::<Vec<_>>();
        if let Some(snapshot) = snapshot { snapshot.commit()?; }
        if rules.sorts.is_empty() && view.presentation.group_key.is_none() {
            return Ok(ordered.into_iter().map(|(item, _)| item).collect());
        }
        ordered.sort_by(|((left, _), a), ((right, _), b)| {
            for ((a, b), rule) in a.iter().zip(b).zip(&rules.sorts) {
                let order = match (a, b) {
                    (Some(a), Some(b)) => {
                        let order = compare_value(rule.value_type(), a, b);
                        if rule.descending { order.reverse() } else { order }
                    }
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                };
                if !order.is_eq() { return order; }
            }
            left.title.cmp(&right.title).then_with(|| left.id.cmp(&right.id))
        });
        Ok(ordered.into_iter().map(|(item, _)| item).collect())
    }

    fn collection_values(&self, collection: i64, key: &str) -> Result<HashMap<i64,String>> {
        let mut statement = self.connection.prepare(
            "SELECT c.note_id,p.value FROM collection_notes c JOIN note_properties p ON p.note_id=c.note_id
             WHERE c.collection_id=?1 AND p.key=?2",
        )?;
        Ok(statement.query_map(params![collection,key], |row| Ok((row.get(0)?,row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn presented_collection_lanes(
        &self, collection: i64, view: Option<&CollectionView>,
    ) -> Result<Vec<(String,Vec<Row>)>> {
        let rows = self.filtered_collection_rows(collection, view)?;
        let group_key = view.and_then(|view| view.presentation.group_key.as_deref());
        let group = group_key.map(|key| self.collection_values(collection,key)).transpose()?;
        if group.is_none() && view.is_some_and(|view| view.layout == CollectionLayout::Table && view.rules.as_ref()
            .is_some_and(|rules| !rules.sorts.is_empty()))
        {
            // Tables and lists preserve global sort order while still displaying the real status.
            let mut ordered = Vec::<(String, Vec<Row>)>::new();
            for (row, status) in rows {
                if let Some((previous, run)) = ordered.last_mut() && *previous == status {
                    run.push(row);
                } else {
                    ordered.push((status, vec![row]));
                }
            }
            return Ok(ordered);
        }
        let mut lanes = BTreeMap::<Option<String>, Vec<Row>>::new();
        for (row, status) in rows {
            let group = match &group {
                Some(group) => group.get(&row.id).filter(|value| !value.is_empty()).cloned(),
                None => Some(status),
            };
            lanes.entry(group).or_default().push(row);
        }
        Ok(lanes.into_iter().map(|(group, rows)|
            (group.map_or_else(|| "未設定".into(), |value|value.replace(['\r','\n','\t']," ")), rows)
        ).collect())
    }

    pub(crate) fn calendar_collection_rows(
        &self, collection: i64, view: &CollectionView, month: Month,
    ) -> Result<BTreeMap<usize, Vec<Row>>> {
        let (start, end) = month.bounds();
        Ok(self.calendar_dated_rows(collection, view, start, end)?.into_iter()
            .map(|(date, rows)| (date.day(), rows)).collect())
    }

    /// A calendar widget supplies a bounded, inclusive viewport (at most 31 days).
    pub(crate) fn calendar_dated_rows(
        &self, collection: i64, view: &CollectionView, period_start: CalendarDay, period_end: CalendarDay,
    ) -> Result<BTreeMap<CalendarDay, Vec<Row>>> {
        if period_end < period_start || period_start.shifted(30).is_some_and(|limit| period_end > limit) {
            return Err("無效的月曆檢視範圍。".into());
        }
        let mut days = BTreeMap::<CalendarDay, Vec<Row>>::new();
        for event in self.scheduled_collection_rows(collection, view, period_start, period_end)? {
            let mut date = event.visible_start;
            while date <= event.visible_end {
                days.entry(date).or_default().push(event.row.clone());
                let Some(next) = date.shifted(1) else { break };
                date = next;
            }
        }
        Ok(days)
    }

    pub(crate) fn timeline_collection_rows(
        &self, collection: i64, view: &CollectionView, month: Month,
    ) -> Result<Vec<ScheduledRow>> {
        if !view.presentation.timeline_mode { return Err("這個檢視沒有時間軸。".into()); }
        let (start, end) = month.bounds();
        let mut rows = self.scheduled_collection_rows(collection, view, start, end)?;
        if view.sort_rules().is_empty() {
            rows.sort_by(|a,b| a.start.cmp(&b.start)
                .then_with(|| a.end.cmp(&b.end))
                .then_with(|| a.row.title.cmp(&b.row.title))
                .then_with(|| a.row.id.cmp(&b.row.id)));
        }
        Ok(rows)
    }

    fn scheduled_collection_rows(
        &self, collection: i64, view: &CollectionView, period_start: CalendarDay, period_end: CalendarDay,
    ) -> Result<Vec<ScheduledRow>> {
        let p = &view.presentation;
        let key = p.calendar_key.as_deref().ok_or("這個檢視沒有日期欄位。")?;
        let starts = self.collection_values(collection, key)?;
        let ends = p.calendar_end_key.as_deref().map(|end| self.collection_values(collection, end)).transpose()?;
        let lower = p.calendar_from.map_or(period_start, |day| day.max(period_start));
        let upper = p.calendar_until.map_or(period_end, |day| day.min(period_end));
        let mut visible = Vec::new();
        for (row, status) in self.filtered_collection_rows(collection, Some(view))? {
            let Some(start) = starts.get(&row.id).and_then(|value| CalendarDay::parse(value)) else { continue };
            let end = ends.as_ref().and_then(|values| values.get(&row.id))
                .and_then(|value| CalendarDay::parse(value)).filter(|end| *end >= start).unwrap_or(start);
            let visible_start = start.max(lower);
            let visible_end = end.min(upper);
            if visible_start <= visible_end {
                visible.push(ScheduledRow { row, status, start, end, visible_start, visible_end });
            }
        }
        Ok(visible)
    }

    pub(crate) fn set_collection_calendar_week(&self,collection:i64,view:&CollectionView,week:CalendarDay) -> Result<()> {
        let p = &view.presentation;
        let old = p.calendar_week_day.ok_or("這個檢視不是週月曆。")?;
        let changed = self.connection.execute(
            "UPDATE collection_views SET calendar_week_day=?5 WHERE collection_id=?1 AND id=?2
             AND calendar_key=?3 AND calendar_week_day=?4",
            params![collection,view.id,&p.calendar_key,old.iso(),week.monday().iso()],
        )?;
        if changed != 1 { return Err("檢視或週次已變更，請重新開啟。".into()); }
        Ok(())
    }

    pub(crate) fn set_collection_calendar_range(
        &self,collection:i64,view:&CollectionView,start:Option<CalendarDay>,end:Option<CalendarDay>,
    ) -> Result<()> {
        validate_calendar_range(start,end)?;
        let p = &view.presentation;
        let key = p.calendar_key.as_deref().ok_or("這個檢視沒有日期欄位。")?;
        let changed = self.connection.execute(
            "UPDATE collection_views SET calendar_from=?6,calendar_until=?7
             WHERE collection_id=?1 AND id=?2 AND calendar_key=?3
             AND calendar_from IS ?4 AND calendar_until IS ?5",
            params![collection,view.id,key,p.calendar_from.map(CalendarDay::iso),p.calendar_until.map(CalendarDay::iso),
                    start.map(CalendarDay::iso),end.map(CalendarDay::iso)],
        )?;
        if changed != 1 { return Err("檢視或日期範圍已變更，請重新開啟。".into()); }
        Ok(())
    }

    pub(crate) fn set_collection_calendar_month(
        &self, collection:i64, view:&CollectionView, month:Month,
    ) -> Result<()> {
        let key = view.presentation.calendar_key.as_deref().ok_or("這個檢視沒有日期欄位。")?;
        if !(1..=9999).contains(&month.year) || !(1..=12).contains(&month.month) {
            return Err("無效的月曆月份。".into());
        }
        let changed = self.connection.execute(
            "UPDATE collection_views SET calendar_month=?5 WHERE collection_id=?1 AND id=?2
             AND calendar_key=?3 AND calendar_month IS ?4",
            params![collection,view.id,key,view.presentation.calendar_month.map(Month::serial),month.serial()],
        )?;
        if changed != 1 { return Err("檢視或月份已變更，請重新開啟。".into()); }
        Ok(())
    }

    pub(crate) fn delete_collection_view(&self, collection: i64, id: i64) -> Result<()> {
        if self.connection.execute("DELETE FROM collection_views WHERE collection_id=?1 AND id=?2",
                                   params![collection,id])? == 0 { return Err("此檢視已不存在。".into()); }
        Ok(())
    }
}

fn collection_view_rows(db: &Connection, collection: i64, id: Option<i64>) -> Result<Vec<CollectionView>> {
    let mut stmt = db.prepare("SELECT id,name,layout,status,keyword,filter_key,filter_kind,filter_value,calendar_key,group_key,sort_key,sort_kind,sort_desc,calendar_month,calendar_week_day,calendar_from,calendar_until,list_mode,calendar_end_key,rules_json,settings_revision,timeline_mode FROM collection_views
                                            WHERE collection_id=?1 AND (?2 IS NULL OR id=?2) ORDER BY name COLLATE NOCASE,id")?;
    let rows = stmt.query_map(params![collection, id], |r| {
        let layout = match r.get_ref(2)?.as_str()? {
            "board" => CollectionLayout::Board, "table" => CollectionLayout::Table,
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        let filter = match (r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,Option<String>>(7)?) {
            (Some(key),Some(kind),Some(value)) => Some(PropertyFilter {key,kind:PropertyType::decode(&kind)?,value}),
            (None,None,None) => None,
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        let date = |column:usize| -> rusqlite::Result<Option<CalendarDay>> {
            r.get::<_,Option<String>>(column)?.map(|value|CalendarDay::parse(&value)
                .ok_or(rusqlite::Error::InvalidQuery)).transpose()
        };
        let presentation = Presentation {
            calendar_key:r.get(8)?, calendar_end_key:r.get(18)?, group_key:r.get(9)?, sort_key:r.get(10)?,
            sort_kind:r.get::<_,Option<String>>(11)?.as_deref().map(PropertyType::decode).transpose()?,
            sort_desc:r.get(12)?,
            calendar_month:r.get::<_,Option<i32>>(13)?.map(|value|
                Month::from_serial(value).ok_or(rusqlite::Error::InvalidQuery)).transpose()?,
            calendar_week_day:date(14)?,calendar_from:date(15)?,calendar_until:date(16)?,list_mode:r.get(17)?,timeline_mode:r.get(21)?,
        };
        Ok(CollectionView { filter, presentation, rules:r.get::<_,Option<String>>(19)?.as_deref().map(ViewRules::decode).transpose()?,
            settings_revision:r.get(20)?, id:r.get(0)?, name:r.get(1)?, layout, status:r.get(3)?, keyword:r.get(4)? })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn collection_view_name(name: &str) -> Result<&str> {
    let name = field(name)?;
    if name.is_empty() { return Err("請輸入檢視名稱。".into()); }
    Ok(name)
}

fn require_current_view(db: &Connection, collection: i64, view: &CollectionView) -> Result<()> {
    if collection_view_rows(db, collection, Some(view.id))?.first() != Some(view) {
        return Err("檢視已變更或移除，請重新開啟。".into());
    }
    Ok(())
}

fn require_available_name(db: &Connection, collection: i64, name: &str, except: Option<i64>) -> Result<()> {
    let taken: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM collection_views WHERE collection_id=?1 AND name=?2 COLLATE NOCASE
                AND (?3 IS NULL OR id<>?3))",
        params![collection, name, except], |row| row.get(0),
    )?;
    if taken { return Err("這個集合已有同名檢視。".into()); }
    Ok(())
}

pub(super) fn validate_collection_field_dates(
    db: &Connection, note: i64, updates: [(&str, Option<CalendarDay>); 2],
) -> Result<()> {
    if updates.iter().all(|(key, _)| key.is_empty()) { return Ok(()); }
    let mut stmt = db.prepare(
        "SELECT DISTINCT v.calendar_key, v.calendar_end_key, s.value, e.value
         FROM collection_notes c JOIN collection_views v ON v.collection_id=c.collection_id
         LEFT JOIN note_properties s ON s.note_id=c.note_id AND s.key=v.calendar_key
         LEFT JOIN note_properties e ON e.note_id=c.note_id AND e.key=v.calendar_end_key
         WHERE c.note_id=?1 AND v.calendar_end_key IS NOT NULL
           AND (?2 IN (v.calendar_key,v.calendar_end_key) OR ?3 IN (v.calendar_key,v.calendar_end_key))",
    )?;
    let mut rows = stmt.query(params![note, updates[0].0, updates[1].0])?;
    while let Some(row) = rows.next()? {
        let date = |key, value| -> rusqlite::Result<Option<CalendarDay>> {
            let key = row.get_ref(key)?.as_str()?;
            if let Some((_, next)) = updates.iter().find(|(name, _)| *name == key) { return Ok(*next); }
            Ok(row.get::<_, Option<String>>(value)?.as_deref().and_then(CalendarDay::parse))
        };
        let (start, end) = (date(0, 2)?, date(1, 3)?);
        if end.is_some() && start.is_none() || start.zip(end).is_some_and(|(start, end)| start > end) {
            return Err("這個變更會讓某個日期檢視的起訖缺漏或顛倒。請一起檢查那組日期欄。".into());
        }
    }
    Ok(())
}

fn validate_end_key(tx: &Transaction<'_>, start: Option<&str>, end: Option<&str>) -> Result<()> {
    if let Some(end) = end {
        if start.is_none_or(|start| start == end) || lookup(tx, end)? != Some(PropertyType::Date) {
            return Err("結束日期需要和起始日期不同，並使用已宣告的日期欄位。".into());
        }
    }
    Ok(())
}

fn compare_value(kind: Option<PropertyType>, a: &str, b: &str) -> Ordering {
    if kind == Some(PropertyType::Number) { decimal_order(a, b) } else { a.cmp(b) }
}

fn validate_calendar_range(start:Option<CalendarDay>,end:Option<CalendarDay>) -> Result<()> {
    if let (Some(start),Some(end)) = (start,end) && start>end {
        return Err("起始日期不能晚於結束日期。".into());
    }
    Ok(())
}

fn decimal_order(a:&str,b:&str) -> std::cmp::Ordering {
    let negative = |v:&str| v.starts_with('-');
    match (negative(a),negative(b)) {
        (true,false) => return std::cmp::Ordering::Less,
        (false,true) => return std::cmp::Ordering::Greater,
        _ => {}
    }
    let reverse = negative(a);
    let a = a.trim_start_matches('-');
    let b = b.trim_start_matches('-');
    let (aw,af) = a.split_once('.').unwrap_or((a,""));
    let (bw,bf) = b.split_once('.').unwrap_or((b,""));
    let order = aw.len().cmp(&bw.len()).then_with(|| aw.cmp(bw)).then_with(|| af.cmp(bf));
    // Canonical decimals omit fractional trailing zeroes, so lexicographic
    // comparison of the fraction already has the correct prefix ordering.
    if reverse { order.reverse() } else { order }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn day(raw: &str) -> CalendarDay { CalendarDay::parse(raw).unwrap() }

    #[test]
    fn relative_calendar_filters_observe_monday_month_and_year_boundaries() -> Result<()> {
        use FilterOperator::*;
        for (operator, today, inside, outside) in [
            (ThisWeek, "2028-12-31", ["2028-12-25", "2028-12-31"], ["2028-12-24", "2029-01-01"]),
            (ThisWeek, "2029-01-01", ["2029-01-01", "2029-01-07"], ["2028-12-31", "2029-01-08"]),
            (PreviousWeek, "2029-01-01", ["2028-12-25", "2028-12-31"], ["2028-12-24", "2029-01-01"]),
            (NextWeek, "2028-12-31", ["2029-01-01", "2029-01-07"], ["2028-12-31", "2029-01-08"]),
            (ThisMonth, "2028-02-29", ["2028-02-01", "2028-02-29"], ["2028-01-31", "2028-03-01"]),
            (PreviousMonth, "2028-03-01", ["2028-02-01", "2028-02-29"], ["2028-01-31", "2028-03-01"]),
            (NextMonth, "2028-12-31", ["2029-01-01", "2029-01-31"], ["2028-12-31", "2029-02-01"]),
        ] {
            assert!(!operator.needs_value());
            assert!(FilterKind::Formula(Some(PropertyType::Date)).allows(operator));
            assert!(!FilterKind::Formula(None).allows(operator));
            for date in inside { assert!(operator.matches_relative(day(date), day(today), None), "{operator:?} {date}"); }
            for date in outside { assert!(!operator.matches_relative(day(date), day(today), None), "{operator:?} {date}"); }
        }
        let first = day("0001-01-01");
        let last = CalendarDay::LAST;
        assert!(ThisWeek.matches_relative(first, first, None));
        assert!(!PreviousWeek.matches_relative(first, first, None));
        assert!(!PreviousMonth.matches_relative(first, first, None));
        assert!(ThisWeek.matches_relative(last, last, None));
        assert!(!NextWeek.matches_relative(last, last, None));
        assert!(!NextMonth.matches_relative(last, last, None));
        let old = r#"[false,[["到期","date",15,"2"],["公式日期","formula-date",14,""]],[]]"#;
        assert_eq!(ViewRules::decode(old)?.encode(), old);
        let invalid_typed = r#"[false,[["工時","number",18,""]],[]]"#;
        assert!(ViewRules::decode(invalid_typed).is_err());
        Ok(())
    }

    #[test]
    fn relative_filters_follow_the_day_at_evaluation_and_keep_typed_formula_results() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("workspace.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let collection = db.create_collection("每日排程")?;
        let mut candidates = Vec::new();
        for (name,date) in [
            ("前兩天",Some("2028-02-27")), ("昨天",Some("2028-02-28")), ("今天",Some("2028-02-29")),
            ("明天",Some("2028-03-01")), ("後天",Some("2028-03-02")), ("舊的錯誤",Some("2028-02-31")), ("空白",None),
        ] {
            let id = db.add(None,name,"")?;
            db.add_to_collection(collection,id,"排程")?;
            if let Some(date) = date { db.compare_set_property(id,"截止",None,Some(date))?; }
            candidates.push(id);
        }
        db.compare_set_property_type("截止",None,Some(PropertyType::Date))?;
        db.add_collection_formula(collection,"日期公式",r#"typed("截止")"#)?;
        let view = db.create_collection_view(collection,"每天動態",CollectionLayout::Table,"","")?;
        let typed = |op,value: &str| FilterRule { key:"截止".into(), kind:FilterKind::Typed(PropertyType::Date),
            operator:op, value:value.into(), argument:None };
        let formula = |op,value: &str| FilterRule { key:"日期公式".into(), kind:FilterKind::Formula(Some(PropertyType::Date)),
            operator:op, value:value.into(), argument:None };
        let invalid = ViewRules { filters: vec![typed(FilterOperator::PreviousDays,"366")], ..Default::default() };
        assert!(db.set_collection_view_rules(collection,&view,invalid).is_err());
        let wrong_kind = ViewRules { filters: vec![FilterRule { kind:FilterKind::Title, ..typed(FilterOperator::Today,"") }],
            ..Default::default() };
        assert!(db.set_collection_view_rules(collection,&view,wrong_kind).is_err());
        let rules = ViewRules { groups: vec![FilterGroup {
            mode:FilterMode::Any,
            filters:vec![typed(FilterOperator::PreviousDays,"1"),formula(FilterOperator::Tomorrow,"")],
            groups: Vec::new(),
        }], ..Default::default() };
        db.set_collection_view_rules(collection,&view,rules.clone())?;
        let saved = db.collection_views(collection)?.remove(0);
        let ids = |db: &Notebook, view: &CollectionView, date: &str| -> Result<Vec<i64>> {
            let mut result = db.filter_collection_rows(collection,Some(view),None,CalendarDay::parse(date))?
                .into_iter().map(|(row,_)| row.id).collect::<Vec<_>>();
            result.sort();
            Ok(result)
        };
        assert_eq!(ids(&db,&saved,"2028-02-29")?,[candidates[1],candidates[3]]);
        let mut problems = RuleProblems::default();
        db.filter_collection_rows(collection,Some(&saved),Some(&mut problems),CalendarDay::parse("2028-02-29"))?;
        assert_eq!(problems.entries.len(),2);
        drop(db);

        let db = Notebook::open(&path,false)?;
        let saved = db.collection_views(collection)?.remove(0);
        assert_eq!(saved.rules(),rules);
        assert_eq!(ids(&db,&saved,"2028-03-01")?,[candidates[2],candidates[4]]);
        assert_eq!(ids(&db,&saved,"2028-03-02")?,[candidates[3]]);
        db.set_collection_view_rules(collection,&saved,ViewRules {
            filters:vec![formula(FilterOperator::NextDays,"2")], ..Default::default()
        })?;
        let saved = db.collection_views(collection)?.remove(0);
        assert_eq!(ids(&db,&saved,"2028-02-29")?,[candidates[3],candidates[4]]);
        db.set_collection_view_rules(collection,&saved,ViewRules {
            filters:vec![typed(FilterOperator::Today,"")], ..Default::default()
        })?;
        let saved = db.collection_views(collection)?.remove(0);
        assert_eq!(ids(&db,&saved,"2028-02-29")?,[candidates[2]]);
        assert!(ids(&db,&saved,"2028-03-03")?.is_empty());
        let calendar_rules = ViewRules {
            filters: vec![typed(FilterOperator::NextMonth,"")],
            groups: vec![FilterGroup {
                filters:vec![formula(FilterOperator::ThisWeek,"")], ..Default::default()
            }], ..Default::default()
        };
        db.set_collection_view_rules(collection,&saved,calendar_rules.clone())?;
        let expected = db.collection_views(collection)?.remove(0);
        assert_eq!(ids(&db,&expected,"2028-02-29")?,[candidates[3],candidates[4]]);
        drop(db);

        let db = Notebook::open(&path,false)?;
        let saved = db.collection_views(collection)?.remove(0);
        assert_eq!(saved.rules(),calendar_rules);
        assert!(ids(&db,&saved,"2028-03-01")?.is_empty());
        assert_eq!(day("2000-03-01").ordinal()-day("2000-02-28").ordinal(),2);
        assert_eq!(day("2100-03-01").ordinal()-day("2100-02-28").ordinal(),1);
        assert_eq!(day("2029-01-01").ordinal()-day("2028-12-31").ordinal(),1);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn saved_derived_sort_uses_live_values_keeps_errors_last_and_reopens() -> Result<()> {
        use SortSource::{Formula, RollupCount, RollupSum};
        let dir = super::super::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let collection = db.create_collection("審片")?;
        let dest = db.create_collection("剪輯")?;
        let a = db.add(None, "A", "")?;
        let b = db.add(None, "B", "")?;
        let zero = db.add(None, "零", "")?;
        let bad = db.add(None, "壞", "")?;
        let hidden = db.add(None, "排除", "")?;
        let high = db.add(None, "同名明細", "")?;
        let low = db.add(None, "同名明細", "")?;
        for (note, rank) in [(a, "5"), (b, "5"), (zero, "0"), (bad, "文字"), (hidden, "999")] {
            db.set_property(note, "順位", Some(rank))?;
            db.add_to_collection(collection, note, if note == hidden { "隱藏" } else { "待處理" })?;
        }
        for (note, hours) in [(high, "10"), (low, "2")] {
            db.set_property(note, "工時", Some(hours))?;
        }
        db.compare_set_property_type("順位", None, Some(PropertyType::Number))?;
        db.set_relation(a, "明細", high, true)?;
        db.set_relation(b, "明細", low, true)?;
        db.add_collection_formula(collection, "計算順位", r#"typed("順位")"#)?;
        let view = db.create_collection_view(collection, "排序", CollectionLayout::Table, "", "")?;
        let filter = FilterRule { key: String::new(), kind: FilterKind::Status,
            operator: FilterOperator::Equal, value: "待處理".into(), argument: None };
        let rule = |key: &str, source, descending| SortRule { key: key.into(), source, descending };
        let rules = ViewRules {
            filters: vec![filter],
            sorts: vec![rule("計算順位", Formula(Some(PropertyType::Number)), true),
                rule("明細", RollupSum("工時".into()), false), rule("明細", RollupCount, true)],
            ..Default::default()
        };
        db.set_collection_view_rules(collection, &view, rules.clone())?;
        let view = db.collection_views(collection)?.remove(0);
        let ids = |db: &Notebook, view: &CollectionView| -> Result<Vec<i64>> {
            Ok(db.filtered_collection_rows(collection, Some(view))?.into_iter().map(|(row, _)| row.id).collect())
        };
        assert_eq!(ids(&db, &view)?, [b, a, zero, bad]);
        let errors = db.collection_rule_problems(collection, &view)?;
        assert_eq!(errors.entries.len(), 1);
        assert!(errors.entries[0].contains(&format!("筆記 #{bad}　排序　公式「計算順位」")));
        assert!(db.copy_collection_view_to(collection, &view, dest, "無對應公式").is_err());
        assert!(db.collection_views(dest)?.is_empty());
        let mut duplicate = rules.clone();
        duplicate.sorts.push(rule("計算順位", Formula(None), false));
        assert!(db.set_collection_view_rules(collection, &view, duplicate).is_err());
        db.set_property(high, "工時", Some("-1"))?;
        assert_eq!(ids(&db, &view)?, [a, b, zero, bad]);
        drop(db);

        let db = Notebook::open(&path, false)?;
        let view = db.collection_views(collection)?.remove(0);
        assert_eq!(view.rules(), rules);
        assert_eq!(ids(&db, &view)?, [a, b, zero, bad]);
        db.set_property(high, "工時", None)?;
        assert_eq!(ids(&db, &view)?, [b, a, zero, bad]);
        assert!(db.collection_rule_problems(collection, &view)?.entries.iter().any(|entry|
            entry.contains(&format!("筆記 #{a}　排序　「明細」的「工時」加總"))));
        db.remove_collection_formula(collection, "計算順位", r#"typed("順位")"#)?;
        assert_eq!(ids(&db, &view)?.len(), 4);
        assert!(db.collection_rule_problems(collection, &view)?.entries.iter().any(|entry|
            entry.contains("找不到集合公式「計算順位」")));
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn derived_sort_encoding_preserves_old_tuples_and_rejects_malformed_rules() -> Result<()> {
        let legacy = r#"[false,[],[["未指定",null,false],["日期","date",true]]]"#;
        assert_eq!(ViewRules::decode(legacy)?.encode(), legacy);
        for bad in [
            r#"[false,[],[["明細","rollup-sum",false]]]"#,
            r#"[false,[],[["明細","rollup-sum",false,""]]]"#,
            r#"[false,[],[["明細","rollup-count",false,"額外"]]]"#,
            r#"[false,[],[["順位","formula-number","false"]]]"#,
            r#"[false,[],[["順位","relation",false]]]"#,
        ] {
            assert!(ViewRules::decode(bad).is_err(), "{bad}");
        }
        let db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let collection = db.create_collection("空集合")?;
        db.add_collection_formula(collection, "日期", r#"date("2028-02-29")"#)?;
        let empty = db.create_collection_view(collection, "空清單", CollectionLayout::Table, "", "")?;
        db.set_collection_view_rules(collection, &empty, ViewRules {
            sorts: vec![SortRule { key: "日期".into(), source: SortSource::Formula(Some(PropertyType::Date)),
                descending: false }], ..Default::default()
        })?;
        let empty = db.collection_views(collection)?.remove(0);
        assert!(db.filtered_collection_rows(collection, Some(&empty))?.is_empty());
        assert!(db.collection_rule_problems(collection, &empty)?.entries.is_empty());
        Ok(())
    }

    #[test]
    fn derived_rules_match_live_ids_typed_values_and_rollups_after_reopen() -> Result<()> {
        use FilterKind::{Formula, Relation, RollupCount, RollupSum};
        use FilterOperator::{Contains, Equal, GreaterOrEqual, IsEmpty};
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!("paperroom-derived-{}-{}.db",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        let mut db = Notebook::open(&path, false)?;
        let collection = db.create_collection("分鏡")?;
        let a = db.add(None, "同名剪接", "")?;
        let b = db.add(None, "同名剪接", "")?;
        let first = db.add(None, "甲", "")?;
        let other_id = db.add(None, "乙", "")?;
        let unchecked = db.add(None, "丙", "")?;
        let empty = db.add(None, "丁", "")?;
        let hold = db.add(None, "戊", "")?;
        let missing = db.add(None, "己", "")?;
        db.compare_set_property(a, "工時", None, Some("12"))?;
        db.compare_set_property(b, "工時", None, Some("8"))?;
        db.compare_set_property_type("完成", None, Some(PropertyType::Checkbox))?;
        for (source, target, flag, status) in [
            (first,Some(a),Some("true"),"Ready"), (other_id,Some(b),Some("false"),"Ready"),
            (unchecked,Some(a),Some("false"),"Ready"), (empty,None,Some("false"),"Ready"),
            (hold,Some(a),Some("false"),"Hold"), (missing,Some(b),None,"Ready"),
        ] {
            db.add_to_collection(collection, source, status)?;
            if let Some(target) = target { db.set_relation(source, "子項", target, true)?; }
            if let Some(flag) = flag { db.compare_set_property(source, "完成", None, Some(flag))?; }
        }
        db.add_collection_formula(collection, "完成檢查", r#"typed("完成")"#)?;
        db.add_collection_formula(collection, "日期格式", r#"date("2028-02-29")"#)?;
        let leaf = |key: &str, kind, op, value: &str, argument: Option<&str>| FilterRule {
            key: key.into(), kind, operator: op, value: value.into(), argument: argument.map(str::to_owned),
        };
        let rules = ViewRules { groups: vec![FilterGroup {
            mode: FilterMode::Any, filters: vec![leaf("子項", RollupSum, GreaterOrEqual, "10", Some("工時"))],
            groups: vec![FilterGroup { filters: vec![
                leaf("完成檢查", Formula(Some(PropertyType::Checkbox)), Equal, "false", None),
                leaf("子項", Relation, Contains, &a.to_string(), None),
            ], ..Default::default() }],
        }], ..Default::default() };
        let view = db.create_collection_view(collection, "衍生條件", CollectionLayout::Table, "Ready", "")?;
        db.set_collection_view_rules(collection, &view, rules.clone())?;
        let view = db.collection_views(collection)?.remove(0);
        assert_eq!(view.rules(), rules);
        let ids = |db: &Notebook, view: &CollectionView| -> Result<Vec<i64>> {
            let mut ids = db.filtered_collection_rows(collection, Some(view))?.into_iter().map(|(row, _)|row.id).collect::<Vec<_>>();
            ids.sort();
            Ok(ids)
        };
        assert_eq!(ids(&db, &view)?, [first,unchecked]);
        let diagnostics = db.collection_rule_problems(collection, &view)?;
        assert_eq!(diagnostics.entries.len(), 1);
        assert!(diagnostics.entries[0].contains(&format!("#{missing}")));
        db.compare_set_property(a, "工時", Some("12"), Some("6"))?;
        assert_eq!(ids(&db, &view)?, [unchecked]);
        let item = db.note(a)?.unwrap();
        db.save(a, item.version, "改名還是同一 ID", &item.body)?;
        assert_eq!(ids(&db, &view)?, [unchecked]);
        let outer = db.connection.unchecked_transaction()?;
        assert_eq!(ids(&db, &view)?, [unchecked]);
        assert_eq!(db.evaluate_note_formula(unchecked, r#"typed("完成")"#)?, "false");
        outer.rollback()?;
        drop(db);

        let mut db = Notebook::open(&path, false)?;
        let view = db.collection_views(collection)?.remove(0);
        assert_eq!(ids(&db, &view)?, [unchecked]);
        db.trash(a)?;
        assert!(ids(&db, &view)?.is_empty());
        db.restore(a)?;
        assert_eq!(ids(&db, &view)?, [unchecked]);

        let zero = ViewRules { filters: vec![
            leaf("子項", RollupCount, Equal, "0", None),
            leaf("子項", RollupSum, Equal, "0", Some("工時")),
            leaf("子項", Relation, IsEmpty, "", None),
            leaf("完成檢查", Formula(Some(PropertyType::Checkbox)), Equal, "false", None),
            leaf("日期格式", Formula(Some(PropertyType::Date)), Equal, "2028-02-29", None),
        ], ..Default::default() };
        db.set_collection_view_rules(collection, &view, zero.clone())?;
        let view = db.collection_views(collection)?.remove(0);
        assert_eq!(ids(&db, &view)?, [empty]);
        let bad_empty = ViewRules { filters: vec![
            leaf("完成檢查", Formula(Some(PropertyType::Checkbox)), IsEmpty, "", None),
        ], ..Default::default() };
        db.set_collection_view_rules(collection, &view, bad_empty)?;
        let view = db.collection_views(collection)?.remove(0);
        assert!(ids(&db, &view)?.is_empty());
        assert_eq!(db.collection_rule_problems(collection, &view)?.entries.len(), 1);

        let bad_type = ViewRules { filters: vec![
            leaf("日期格式", Formula(None), Equal, "2028-02-29", None),
        ], ..Default::default() };
        db.set_collection_view_rules(collection, &view, bad_type)?;
        let view = db.collection_views(collection)?.remove(0);
        assert!(ids(&db, &view)?.is_empty());
        assert!(db.collection_rule_problems(collection, &view)?.entries[0].contains("型別"));

        let bad_sum = ViewRules { filters: vec![leaf("子項", RollupSum, Equal, "0", Some("工時"))], ..Default::default() };
        db.set_collection_view_rules(collection, &view, bad_sum)?;
        let view = db.collection_views(collection)?.remove(0);
        db.compare_set_property(b, "工時", Some("8"), None)?;
        assert_eq!(ids(&db, &view)?, [empty]);
        assert_eq!(db.collection_rule_problems(collection, &view)?.entries.len(), 2);
        let malformed = r#"[false,[["子項","rollup-sum",0,"0"]],[]]"#;
        assert!(ViewRules::decode(malformed).is_err());
        drop(db);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn nested_filters_keep_v15_rules_and_reject_malformed_groups() -> Result<()> {
        let legacy = r#"[true,[["工時","number",4,"2"],["截止","date",2,"2028-03-01"]],[["工時","number",true]]]"#;
        assert_eq!(ViewRules::decode(legacy)?.encode(), legacy);
        let empty_nested = r#"[true,[],[],[[false,[],[]]]]"#;
        assert!(ViewRules::decode(empty_nested).is_err());
        let invalid_text = r#"[true,[["","title",8,""]],[],[]]"#;
        assert!(ViewRules::decode(invalid_text).is_err());
        let invalid_operator = r#"[true,[["工時","number",8,"2"]],[],[]]"#;
        assert!(ViewRules::decode(invalid_operator).is_err());
        let mut level = json!([false, [["", "title", 0, "甲"]], []]);
        for _ in 1..MAX_FILTER_GROUP_DEPTH {
            level = json!([false, [], [level]]);
        }
        assert!(ViewRules::decode(&json!([false, [], [], [level.clone()]]).to_string()).is_ok());
        assert!(ViewRules::decode(&json!([false, [], [], [[false, [], [level]]]]).to_string()).is_err());
        Ok(())
    }

    #[test]
    fn mixed_filter_groups_reopen_copy_and_match_live_values_without_widening() -> Result<()> {
        use FilterKind::{Status, Text, Title, Typed};
        use FilterOperator::{Contains, Equal, GreaterOrEqual, StartsWith};
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!("paperroom-nested-filter-{}-{}.db",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        let mut db = Notebook::open(&path, false)?;
        let collection = db.create_collection("分鏡")?;
        let destination = db.create_collection("剪輯")?;
        let direct = db.add(None, "Alpha 鏡頭", "")?;
        let nested = db.add(None, "場次乙", "")?;
        let missing = db.add(None, "場次丙", "")?;
        let wrong_status = db.add(None, "Alpha 外景", "")?;
        let failing_deep = db.add(None, "場次戊", "")?;
        for (note, status) in [
            (direct,"Ready"), (nested,"Ready"), (missing,"Ready"), (wrong_status,"Hold"), (failing_deep,"Ready"),
        ] { db.add_to_collection(collection, note, status)?; }
        db.add_to_collection(destination, nested, "Ready")?;
        for (field, kind) in [("開始",PropertyType::Date), ("工時",PropertyType::Number), ("完成",PropertyType::Checkbox)] {
            db.compare_set_property_type(field, None, Some(kind))?;
        }
        for note in [direct,nested,missing,wrong_status,failing_deep] {
            for (key,value) in [
                ("開始","2028-02-29"),
                ("工時",if note == nested { "002.500" } else { "1" }),
                ("完成","false"),
            ] { db.compare_set_property(note, key, None, Some(value))?; }
        }
        for note in [nested, failing_deep] {
            db.compare_set_property(note, "備註", None, Some("文字 A%_b ' 字面"))?;
        }
        let condition = |key: &str, kind, operator, value: &str|
            FilterRule { key: key.into(), kind, operator, value: value.into(), argument: None };
        let rules = ViewRules {
            filters: vec![condition("", Status, Equal, "ready")],
            groups: vec![FilterGroup {
                mode: FilterMode::Any, filters: vec![condition("", Title, StartsWith, "alpha")],
                groups: vec![FilterGroup {
                    mode: FilterMode::All,
                    filters: vec![condition("備註", Text, Contains, "%_B '"),
                                  condition("開始", Typed(PropertyType::Date), Equal, "2028-02-29")],
                    groups: vec![FilterGroup {
                        mode: FilterMode::Any,
                        filters: vec![condition("工時", Typed(PropertyType::Number), GreaterOrEqual, "2.5"),
                                      condition("完成", Typed(PropertyType::Checkbox), Equal, "true")],
                        groups: Vec::new(),
                    }],
                }],
            }],
            ..Default::default()
        };
        let view = db.save_presented_collection_view(collection, "巢狀日期", CollectionLayout::Table, "", "", None,
            Presentation { calendar_key: Some("開始".into()), calendar_month: Month::parse("2028-02"),
                timeline_mode: true, ..Default::default() })?;
        let invalid = ViewRules { mode: FilterMode::Any, groups: vec![FilterGroup::default()], ..rules.clone() };
        assert!(db.set_collection_view_rules(collection, &view, invalid).is_err());
        db.set_collection_view_rules(collection, &view, rules.clone())?;
        assert!(db.set_collection_view_rules(collection, &view, ViewRules::default()).is_err());
        let saved = db.collection_views(collection)?.remove(0);
        assert_eq!(saved.rules(), rules);
        let mut over_budget = rules.clone();
        over_budget.groups = vec![rules.groups[0].clone(); 6];
        assert!(db.set_collection_view_rules(collection, &saved, over_budget).is_err());
        db.copy_collection_view_to(collection, &saved, destination, "保留分組")?;
        drop(db);

        let db = Notebook::open(&path, false)?;
        let saved = db.collection_views(collection)?.remove(0);
        let target = db.collection_views(destination)?.remove(0);
        assert_eq!(target.rules(), rules);
        let month = Month::parse("2028-02").unwrap();
        let mut chosen = db.timeline_collection_rows(collection, &saved, month)?.into_iter()
            .map(|row| row.row.id).collect::<Vec<_>>();
        chosen.sort();
        assert_eq!(chosen, [direct,nested]);
        assert_eq!(db.timeline_collection_rows(destination, &target, month)?[0].row.id, nested);
        db.compare_set_property(nested, "工時", Some("002.500"), Some("1.5"))?;
        assert!(db.timeline_collection_rows(destination, &target, month)?.is_empty());
        db.compare_set_property(nested, "完成", Some("false"), Some("true"))?;
        assert_eq!(db.timeline_collection_rows(destination, &target, month)?[0].row.id, nested);
        db.compare_set_property_type("備註", None, Some(PropertyType::Number))?;
        assert!(db.set_collection_view_rules(collection, &saved, rules).is_err());
        assert_eq!(db.timeline_collection_rows(destination, &target, month)?[0].row.id, nested);
        drop(db);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn timeline_gesture_dates_hold_duration_and_reject_inverted_ranges() {
        use ScheduleAdjustment::*;
        let start = day("2028-02-28");
        let end = day("2028-03-01");
        assert_eq!(Move.apply(start, Some(end), 1), Some((day("2028-02-29"), Some(day("2028-03-02")))));
        assert_eq!(Move.apply(start, None, 1), Some((day("2028-02-29"), None)));
        assert_eq!(Start.apply(start, Some(end), 2), Some((end, Some(end))));
        assert_eq!(Start.apply(start, Some(end), 3), None);
        assert_eq!(End.apply(start, Some(end), -3), None);
        assert_eq!(End.apply(start, None, 1), Some((start, Some(day("2028-02-29")))));
        assert_eq!(End.apply(start, None, 0), Some((start, None)));
        assert_eq!(Move.apply(day("0001-01-01"), None, -1), None);
        assert_eq!(Move.apply(start, Some(CalendarDay::LAST), 1), None);
    }

    #[test]
    fn direct_schedule_and_saved_filters_respect_shared_ranges_and_stale_snapshots() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!("paperroom-timeline-edit-{}-{}.db",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        let mut db = Notebook::open(&path, false)?;
        let month = Month::parse("2028-02").unwrap();
        let collection = db.create_collection("分鏡")?;
        let note = db.add(None, "鏡頭 甲", "")?;
        let second = db.add(None, "鏡頭 乙", "")?;
        db.add_to_collection(collection, note, "攝影")?;
        db.add_to_collection(collection, second, "攝影")?;
        for key in ["開始", "結束", "下階段"] { db.compare_set_property_type(key, None, Some(PropertyType::Date))?; }
        for (note, start, end, dependency) in [
            (note, " 2028-02-26 ", "2028-02-28", "2028-03-01"),
            (second, "2028-02-24", "2028-02-26", "2028-03-02"),
        ] {
            for (key, value) in [("開始", start), ("結束", end), ("下階段", dependency)] {
                db.compare_set_property(note, key, None, Some(value))?;
            }
        }
        let schedule = db.save_presented_collection_view(collection, "A", CollectionLayout::Table, "", "", None,
            Presentation { calendar_key: Some("開始".into()), calendar_end_key: Some("結束".into()),
                calendar_month: Some(month), timeline_mode: true, ..Default::default() })?;
        let previous = db.save_presented_collection_view(collection, "B", CollectionLayout::Table, "", "", None,
            Presentation { calendar_key: Some("結束".into()), calendar_end_key: Some("下階段".into()),
                calendar_month: Some(month), ..Default::default() })?;
        let rules = ViewRules {
            mode: FilterMode::Any,
            filters: vec![FilterRule { key:"下階段".into(), kind:FilterKind::Typed(PropertyType::Date),
                operator:FilterOperator::GreaterOrEqual, value:"2028-03-01".into(), argument: None }],
            ..Default::default()
        };
        db.set_collection_view_rules(collection, &schedule, rules.clone())?;
        assert!(db.set_collection_basic_filter(collection, &schedule, "攝影", "甲").is_err());
        let current = db.collection_views(collection)?.remove(0);
        assert!(db.set_collection_basic_filter(collection, &current, "攝影", "bad\nvalue").is_err());
        db.set_collection_basic_filter(collection, &current, " 攝影 ", " 甲 ")?;
        assert!(db.set_collection_row_dates(collection, &current, note, [Some(" 2028-02-26 "), Some("2028-02-28")],
            [Some(day("2028-02-26")), Some(day("2028-02-29"))]).is_err());
        let current = db.collection_views(collection)?.remove(0);
        assert_eq!(current.rules(), rules);
        let rows = db.timeline_collection_rows(collection, &current, month)?;
        assert_eq!(rows.iter().map(|row| row.row.id).collect::<Vec<_>>(), [note]);
        assert!(db.set_collection_row_dates(collection, &current, note, [Some(" 2028-02-26 "), Some("2028-02-28")],
            [Some(day("2028-02-27")), Some(day("2028-03-02"))]).is_err());
        assert!(db.set_collection_row_dates(collection, &current, note, [Some(" 2028-02-26 "), Some("2028-02-27")],
            [Some(day("2028-02-26")), Some(day("2028-02-29"))]).is_err());
        assert_eq!(db.timeline_collection_rows(collection, &current, month)?[0].end, day("2028-02-28"));
        db.set_collection_row_dates(collection, &current, note, [Some(" 2028-02-26 "), Some("2028-02-28")],
            [Some(day("2028-02-26")), Some(day("2028-02-29"))])?;
        assert!(db.properties(note)?.iter().any(|(key,value)| key == "開始" && value == " 2028-02-26 "));
        assert!(db.calendar_collection_rows(collection, &previous, month)?[&29].iter().any(|row| row.id==note));
        drop(db);

        let db = Notebook::open(&path, false)?;
        let current = db.collection_views(collection)?.remove(0);
        assert_eq!(current.rules(), rules);
        assert_eq!((current.status.as_str(), current.keyword.as_str()), ("攝影", "甲"));
        assert_eq!(db.timeline_collection_rows(collection, &current, month)?[0].end, day("2028-02-29"));
        db.set_collection_basic_filter(collection, &current, "", "")?;
        let updated = db.collection_views(collection)?.remove(0);
        assert_eq!(updated.rules(), rules);
        assert_eq!(db.timeline_collection_rows(collection, &updated, month)?.len(), 2);
        drop(db);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn timeline_reuses_dates_and_copy_uses_destination_members_after_v13_reopen() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!("paperroom-timeline-{}-{}.db",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        let mut db = Notebook::open(&path, false)?;
        assert!(Month::parse("2028-2").is_none());
        assert!(Month::parse("0000-01").is_none());
        assert!(Month::parse("2028-13").is_none());
        let feb = Month::parse("2028-02").unwrap();
        let march = feb.shifted(1);
        assert_eq!(march.iso(), "2028-03");
        let source = db.create_collection("分鏡")?;
        let destination = db.create_collection("剪輯")?;
        for key in ["開始", "結束"] { db.compare_set_property_type(key, None, Some(PropertyType::Date))?; }
        let first = db.add(None, "跨月鏡頭", "")?;
        let second = db.add(None, "剪輯備份", "")?;
        db.add_to_collection(source, first, "製作")?;
        db.add_to_collection(destination, second, "製作")?;
        for (note, start, end) in [(first, "2028-02-27", Some("2028-03-03")), (second, "2028-03-02", None)] {
            db.compare_set_property(note, "開始", None, Some(start))?;
            if let Some(end) = end { db.compare_set_property(note, "結束", None, Some(end))?; }
        }
        let calendar = db.save_presented_collection_view(source, "A 月曆", CollectionLayout::Table, "製作", "", None,
            Presentation { calendar_key: Some("開始".into()), calendar_end_key: Some("結束".into()),
                calendar_month: Some(feb), ..Default::default() })?;
        super::super::tests::remove_v14_schema(&db.connection)?;
        db.connection.pragma_update(None, "user_version", 13)?;
        drop(db);

        let db = Notebook::open(&path, false)?;
        let old = db.collection_views(source)?.remove(0);
        assert_eq!(old, calendar);
        assert!(!old.presentation.timeline_mode);
        assert_eq!(db.calendar_collection_rows(source, &old, feb)?.keys().copied().collect::<Vec<_>>(), [27, 28, 29]);
        assert!(db.save_presented_collection_view(source, "沒有月份", CollectionLayout::Table, "", "", None,
            Presentation { calendar_key: Some("開始".into()), timeline_mode: true, ..Default::default() }).is_err());
        let timeline = db.save_presented_collection_view(source, "B 時間軸", CollectionLayout::Table, "製作", "", None,
            Presentation { calendar_key: Some("開始".into()), calendar_end_key: Some("結束".into()),
                calendar_month: Some(feb), timeline_mode: true, ..Default::default() })?;
        let rows = db.timeline_collection_rows(source, &timeline, feb)?;
        let event = &rows[0];
        assert_eq!(rows.len(), 1);
        assert_eq!(event.row.id, first);
        assert_eq!(event.status, "製作");
        assert_eq!((event.start, event.end), (day("2028-02-27"), day("2028-03-03")));
        assert_eq!((event.visible_start, event.visible_end), (day("2028-02-27"), day("2028-02-29")));
        db.set_collection_calendar_month(source, &timeline, march)?;
        assert!(db.copy_collection_view_to(source, &timeline, destination, "過期").is_err());
        let timeline = db.collection_views(source)?.remove(1);
        let clipped = db.timeline_collection_rows(source, &timeline, march)?;
        assert_eq!((clipped[0].visible_start, clipped[0].visible_end), (day("2028-03-01"), day("2028-03-03")));

        let rules = ViewRules { sorts: vec![SortRule {
            key: "開始".into(), source: SortSource::Property(Some(PropertyType::Date)), descending: true,
        }], ..Default::default() };
        db.set_collection_view_rules(source, &timeline, rules.clone())?;
        let timeline = db.collection_views(source)?.remove(1);
        assert!(db.copy_collection_view_to(source, &timeline, -999, "不存在").is_err());
        let copy = db.copy_collection_view_to(source, &timeline, destination, "本地套用")?;
        assert!(db.copy_collection_view_to(source, &timeline, destination, "本地套用").is_err());
        db.set_collection_row_dates(source, &timeline, first, [Some("2028-02-27"), Some("2028-03-03")],
            [Some(day("2028-02-29")), Some(day("2028-03-02"))])?;
        assert_eq!(db.calendar_collection_rows(source, &old, feb)?.keys().copied().collect::<Vec<_>>(), [29]);
        drop(db);

        let db = Notebook::open(&path, false)?;
        let timeline = db.collection_views(source)?.remove(1);
        let copied = db.collection_views(destination)?.remove(0);
        assert_eq!(timeline.presentation.calendar_month, Some(march));
        assert_eq!(copied.id, copy);
        assert_eq!(copied.presentation, timeline.presentation);
        assert_eq!(copied.rules(), rules);
        let destination_rows = db.timeline_collection_rows(destination, &copied, march)?;
        assert_eq!(destination_rows.len(), 1);
        assert_eq!(destination_rows[0].row.id, second);
        assert_eq!(destination_rows[0].visible_start, destination_rows[0].visible_end);
        let source_rows = db.timeline_collection_rows(source, &timeline, march)?;
        assert_eq!(source_rows[0].visible_end, day("2028-03-02"));
        db.set_collection_calendar_range(destination, &copied, Some(day("2028-03-03")), None)?;
        let copied = db.collection_views(destination)?.remove(0);
        assert!(db.timeline_collection_rows(destination, &copied, march)?.is_empty());
        assert_eq!(db.timeline_collection_rows(source, &timeline, march)?.len(), 1);
        drop(db);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn rename_and_duplicate_keep_saved_recipes_and_reopen_the_same_members() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!("paperroom-view-copy-{}-{}.db",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        let mut db = Notebook::open(&path, false)?;
        let collection = db.create_collection("分鏡")?;
        let elsewhere = db.create_collection("外部集合")?;
        let note = db.add(None, "鏡頭甲", "合成資料")?;
        db.add_to_collection(collection, note, "進行中")?;
        for (key, kind, value) in [
            ("開始", PropertyType::Date, "2028-02-28"), ("結束", PropertyType::Date, "2028-03-02"),
            ("工時", PropertyType::Number, "002.00"),
        ] {
            db.compare_set_property_type(key, None, Some(kind))?;
            db.compare_set_property(note, key, None, Some(value))?;
        }
        let view = db.save_presented_collection_view(collection, "A", CollectionLayout::Table, "進行中", "鏡頭", Some(PropertyFilter {
            key: "工時".into(), kind: PropertyType::Number, value: "+02.000".into(),
        }), Presentation {
            calendar_key: Some("開始".into()), calendar_end_key: Some("結束".into()), group_key: Some("工時".into()),
            sort_key: Some("工時".into()), sort_desc: true, calendar_month: Some(Month { year: 2028, month: 2 }),
            calendar_from: Some(day("2028-02-28")), calendar_until: Some(day("2028-03-03")), ..Default::default()
        })?;
        let mut rules = view.rules();
        rules.mode = FilterMode::Any;
        db.set_collection_view_rules(collection, &view, rules)?;
        assert!(db.duplicate_collection_view(collection, &view, "B").is_err());
        let current = db.collection_views(collection)?.remove(0);
        db.set_collection_calendar_month(collection, &current, Month { year: 2028, month: 3 })?;
        assert!(db.rename_collection_view(collection, &current, "未套用").is_err());
        let current = db.collection_views(collection)?.remove(0);
        assert!(db.rename_collection_view(elsewhere, &current, "跨集合").is_err());
        assert!(db.duplicate_collection_view(collection, &current, " a ").is_err());
        assert!(db.rename_collection_view(collection, &current, "　").is_err());
        let copy = db.duplicate_collection_view(collection, &current, " B ")?;
        assert_ne!(copy, current.id);
        assert!(db.rename_collection_view(collection, &current, "b").is_err());
        db.rename_collection_view(collection, &current, "a")?;
        assert!(db.rename_collection_view(collection, &current, "過期").is_err());

        drop(db);
        let db = Notebook::open(&path, false)?;
        let views = db.collection_views(collection)?;
        let mut expected = current.clone();
        expected.name = "a".into();
        expected.settings_revision += 1;
        assert_eq!(views[0], expected);
        expected.id = copy;
        expected.name = "B".into();
        expected.settings_revision = 0;
        assert_eq!(views[1], expected);
        assert_eq!(views[1].filter.as_ref().unwrap().value, "+02.000");
        assert_eq!(db.collection_rows(collection)?.len(), 1);
        for view in &views {
            let march = Month { year: 2028, month: 3 };
            let result = db.calendar_collection_rows(collection, view, march)?;
            assert_eq!(result.keys().copied().collect::<Vec<_>>(), [1, 2]);
            assert!(result.values().all(|rows| rows.len() == 1 && rows[0].id == note));
        }
        db.set_collection_view_rules(collection, &views[1], ViewRules::default())?;
        assert_eq!(db.collection_views(collection)?[0], views[0]);
        drop(db);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn direct_row_fields_validate_types_conflicts_and_shared_calendar_pairs() -> Result<()> {
        let mut db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let source = db.create_collection("表格")?;
        let calendar = db.create_collection("排程")?;
        let note = db.add(None, "鏡頭甲", "")?;
        db.add_to_collection(source, note, "製作")?;
        db.add_to_collection(calendar, note, "日程")?;
        db.compare_set_property(note, "工時", None, Some("舊字串"))?;
        for (key, kind) in [("開始", PropertyType::Date), ("結束", PropertyType::Date),
                              ("工時", PropertyType::Number), ("完成", PropertyType::Checkbox)] {
            db.compare_set_property_type(key, None, Some(kind))?;
        }
        let view = db.save_presented_collection_view(calendar, "排程", CollectionLayout::Table, "", "", None, Presentation {
            calendar_key: Some("開始".into()), calendar_end_key: Some("結束".into()), ..Default::default()
        })?;
        let apply = |key, kind, previous, value| db.edit_collection_field(source, note, "製作", FieldEdit { key, kind, previous, value });
        apply("工時", PropertyType::Number, Some("舊字串"), Some("舊字串"))?;
        assert!(apply("工時", PropertyType::Number, Some("舊字串"), Some("NaN")).is_err());
        apply("工時", PropertyType::Number, Some("舊字串"), Some("002.500"))?;
        assert!(apply("工時", PropertyType::Number, Some("舊字串"), None).is_err());
        assert_eq!(db.properties(note)?[0].1, "002.500");
        let filter = PropertyFilter { key: "工時".into(), kind: PropertyType::Number, value: "2.5".into() };
        let table = db.save_collection_view(source, "已排程", CollectionLayout::Table, "", "", Some(filter))?;
        assert_eq!(db.filtered_collection_rows(source, Some(&table))?[0].0.id, note);
        apply("完成", PropertyType::Checkbox, None, Some("true"))?;
        apply("完成", PropertyType::Checkbox, Some("true"), None)?;
        assert!(db.properties(note)?.iter().all(|(key, _)| key != "完成"));
        assert!(apply("結束", PropertyType::Date, None, Some("2028-03-02")).is_err());
        apply("開始", PropertyType::Date, None, Some("2028-02-29"))?;
        apply("結束", PropertyType::Date, None, Some("2028-03-02"))?;
        assert!(apply("開始", PropertyType::Date, Some("2028-02-29"), None).is_err());
        assert!(apply("結束", PropertyType::Date, Some("2028-03-02"), Some("2028-02-28")).is_err());
        assert_eq!(db.calendar_collection_rows(calendar, &view, Month { year: 2028, month: 3 })?.len(), 2);
        assert!(apply("工時", PropertyType::Checkbox, Some("002.500"), Some("false")).is_err());
        db.move_collection_note(source, note, "製作", "完成")?;
        assert!(apply("工時", PropertyType::Number, Some("002.500"), None).is_err());
        assert_eq!(db.filtered_collection_rows(source, Some(&table))?[0].0.id, note);
        db.trash(note)?;
        assert!(db.edit_collection_field(source, note, "完成", FieldEdit {
            key: "工時", kind: PropertyType::Number, previous: Some("002.500"), value: None,
        }).is_err());
        Ok(())
    }

    #[test]
    fn calendar_spans_viewports_backs_up_and_rejects_bad_or_stale_pairs_atomically() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!("paperroom-range-{}-{}.db",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        let backup = path.with_extension("backup");
        let mut db = Notebook::open(&path, false)?;
        let id = db.create_collection("分鏡排程")?;
        let note = db.add(None, "跨月鏡頭", "")?;
        db.add_to_collection(id, note, "待處理")?;
        for key in ["開始", "結束"] { db.compare_set_property_type(key, None, Some(PropertyType::Date))?; }
        let make_calendar = |name, week| db.save_presented_collection_view(
            id, name, CollectionLayout::Table, "", "", None, Presentation {
                calendar_key: Some("開始".into()), calendar_week_day: week, ..Default::default()
            },
        );
        let month = make_calendar("A 月曆", None)?;
        let week = make_calendar("B 週曆", Some(day("2028-02-28")))?;
        assert!(db.set_collection_end_key(id, &month, Some("開始")).is_err());
        assert!(db.set_collection_end_key(id, &month, Some("不存在")).is_err());
        db.set_collection_end_key(id, &month, Some("結束"))?;
        db.set_collection_end_key(id, &week, Some("結束"))?;
        let views = db.collection_views(id)?;
        let (month, week) = (&views[0], &views[1]);

        for pair in [[None, Some(day("2028-03-02"))], [Some(day("2028-03-02")), Some(day("2028-02-28"))]] {
            assert!(db.set_collection_row_dates(id, month, note, [None, None], pair).is_err());
        }
        assert!(db.properties(note)?.is_empty());
        db.set_collection_row_dates(id, month, note, [None, None], [Some(day("2028-02-27")), Some(day("2028-03-02"))])?;
        assert!(db.set_collection_row_dates(id, week, note, [None, None], [Some(day("2028-02-28")), None]).is_err());
        let feb = Month { year: 2028, month: 2 };
        let march = feb.shifted(1);
        assert_eq!(db.calendar_collection_rows(id, month, feb)?.keys().copied().collect::<Vec<_>>(), [27, 28, 29]);
        assert_eq!(db.calendar_collection_rows(id, month, march)?.keys().copied().collect::<Vec<_>>(), [1, 2]);
        let days = db.calendar_dated_rows(id, week, day("2028-02-28"), day("2028-03-05"))?;
        assert_eq!(days.keys().map(|key| key.iso()).collect::<Vec<_>>(), ["2028-02-28", "2028-02-29", "2028-03-01", "2028-03-02"]);
        assert!(days.values().all(|rows| rows.len() == 1 && rows[0].id == note));
        db.set_collection_calendar_range(id, week, Some(day("2028-02-29")), Some(day("2028-03-01")))?;
        db.backup(&backup)?;
        drop(db);
        let db = Notebook::open(&backup, false)?;
        let views = db.collection_views(id)?;
        let (month, week) = (&views[0], &views[1]);
        let days = db.calendar_dated_rows(id, week, day("2028-02-28"), day("2028-03-05"))?;
        assert_eq!(days.keys().map(|key| key.iso()).collect::<Vec<_>>(), ["2028-02-29", "2028-03-01"]);

        db.compare_set_property(note, "結束", Some("2028-03-02"), Some("2028-02-26"))?;
        assert_eq!(db.calendar_collection_rows(id, month, feb)?.keys().copied().collect::<Vec<_>>(), [27]);
        assert!(db.calendar_collection_rows(id, month, march)?.is_empty());
        db.set_collection_row_dates(
            id, week, note, [Some("2028-02-27"), Some("2028-02-26")], [None, None],
        )?;
        assert!(db.properties(note)?.is_empty());
        drop(db);
        std::fs::remove_file(backup)?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn composable_rules_order_and_filter_live_rows_in_all_saved_layouts() -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let path = std::env::temp_dir().join(format!("paperroom-rules-{}-{}.db",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        let mut db = Notebook::open(&path, false)?;
        let id = db.create_collection("拍攝")?;
        let base = db.add(None, "鏡頭甲", "")?;
        let tiebreak = db.add(None, "鏡頭乙", "")?;
        let bigger = db.add(None, "鏡頭丙", "")?;
        let removed = db.add(None, "外景", "")?;
        let missing = db.add(None, "鏡頭無", "")?;
        let invalid = db.add(None, "鏡頭舊", "")?;
        for note in [base, tiebreak, bigger, removed, missing, invalid] {
            db.add_to_collection(id, note, if note == tiebreak { "A" } else { "B" })?;
            db.compare_set_property(note, "日期", None, Some("2028-02-29"))?;
        }
        db.compare_set_property(invalid, "工時", None, Some("舊值"))?;
        for (key, kind) in [("工時", PropertyType::Number), ("日期", PropertyType::Date),
                            ("優先", PropertyType::Number), ("已排程", PropertyType::Checkbox)] {
            db.compare_set_property_type(key, None, Some(kind))?;
        }
        for (note, hours, priority, scheduled) in [
            (base, "9007199254740992", "2", "true"), (tiebreak, "9007199254740992.0000", "10", "true"),
            (bigger, "9007199254740993", "7", "true"), (removed, "9007199254740994", "9", "true"),
        ] {
            for (key, value) in [("工時", hours), ("優先", priority), ("已排程", scheduled)] {
                db.compare_set_property(note, key, None, Some(value))?;
            }
        }
        let hours = FilterRule {
            key: "工時".into(), kind: FilterKind::Typed(PropertyType::Number), operator: FilterOperator::GreaterOrEqual,
            value: "+9007199254740992.00".into(), argument: None
        };
        let scheduled = FilterRule { key: "已排程".into(), kind: FilterKind::Typed(PropertyType::Checkbox),
            operator: FilterOperator::Equal, value: "true".into(), argument: None };
        let rules = ViewRules {
            mode: FilterMode::All, filters: vec![hours, scheduled], groups: Vec::new(),
            sorts: vec![
                SortRule { key: "工時".into(), source: SortSource::Property(Some(PropertyType::Number)), descending: false },
                SortRule { key: "優先".into(), source: SortSource::Property(Some(PropertyType::Number)), descending: true },
            ],
        };
        for (name, layout, presentation) in [
            ("A 看板", CollectionLayout::Board, Presentation::default()),
            ("B 表格", CollectionLayout::Table, Presentation::default()),
            ("C 清單", CollectionLayout::Table, Presentation { list_mode: true, ..Default::default() }),
            ("D 月曆", CollectionLayout::Table, Presentation { calendar_key: Some("日期".into()), ..Default::default() }),
            ("E 週曆", CollectionLayout::Table, Presentation { calendar_key: Some("日期".into()),
                calendar_week_day: Some(day("2028-02-28")), ..Default::default() }),
        ] {
            let view = db.save_presented_collection_view(id, name, layout, "", "鏡頭", None, presentation)?;
            db.set_collection_view_rules(id, &view, rules.clone())?;
            assert!(db.set_collection_view_rules(id, &view, ViewRules::default()).is_err());
        }
        drop(db);
        let db = Notebook::open(&path, false)?;
        let views = db.collection_views(id)?;
        for view in &views {
            assert_eq!(view.rules().filters[0].value, "9007199254740992");
            assert_eq!(db.filtered_collection_rows(id, Some(view))?.into_iter().map(|(row,_)|row.id)
                .collect::<Vec<_>>(), [tiebreak, base, bigger]);
            let lanes = db.presented_collection_lanes(id, Some(view))?;
            assert_eq!(lanes.into_iter().flat_map(|(_,rows)|rows).map(|row|row.id)
                .collect::<Vec<_>>(), [tiebreak, base, bigger]);
            if view.presentation.calendar_key.is_some() {
                let days = db.calendar_dated_rows(id, view, day("2028-02-28"), day("2028-03-05"))?;
                assert_eq!(days[&day("2028-02-29")].iter().map(|row|row.id).collect::<Vec<_>>(), [tiebreak, base, bigger]);
            }
        }
        let mut any = rules.clone();
        any.mode = FilterMode::Any;
        any.filters[1].operator = FilterOperator::NotEqual;
        any.filters.push(FilterRule { key: "工時".into(), kind: FilterKind::Typed(PropertyType::Number),
            operator: FilterOperator::IsEmpty, value: String::new(), argument: None });
        db.set_collection_view_rules(id, &views[1], any.clone())?;
        let table = &db.collection_views(id)?[1];
        assert_eq!(db.filtered_collection_rows(id, Some(table))?.into_iter().map(|(row,_)|row.id)
            .collect::<Vec<_>>(), [tiebreak, base, bigger, missing]);
        assert_eq!(table.rules().mode, FilterMode::Any);
        any.sorts.push(any.sorts[0].clone());
        assert!(db.set_collection_view_rules(id, table, any).is_err());
        assert_eq!(&db.collection_views(id)?[1], table);
        db.compare_set_property(bigger, "工時", Some("9007199254740993"), Some("2"))?;
        assert_eq!(db.filtered_collection_rows(id, Some(&views[2]))?.into_iter().map(|(row,_)|row.id)
            .collect::<Vec<_>>(), [tiebreak, base]);
        drop(db);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn saved_calendar_and_typed_group_order_follow_live_members_after_reopen() -> Result<()> {
        let path = std::env::temp_dir().join(format!("paperroom-cal-{}-{}.db",
            std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        let feb = Month {year:2028,month:2};
        assert_eq!(feb.days(),29);
        assert_eq!(feb.sunday_offset(),2);
        assert_eq!(Month{year:2027,month:12}.shifted(1),Month{year:2028,month:1});
        assert_eq!(decimal_order("-100.15","-2.5"),std::cmp::Ordering::Less);
        assert_eq!(decimal_order("2","10"),std::cmp::Ordering::Less);
        let (collection,a,z,empty,calendar_id,grouped_id) = {
            let mut db = Notebook::open(&path,false)?;
            let collection = db.create_collection("工作")?;
            let a = db.add(None,"A","")?;
            let z = db.add(None,"Z","")?;
            let empty = db.add(None,"未規劃","")?;
            for note in [a,z,empty] { db.add_to_collection(collection,note,"待辦")?; }
            db.compare_set_property_type("日期",None,Some(PropertyType::Date))?;
            db.compare_set_property_type("優先",None,Some(PropertyType::Number))?;
            for (note,date,number) in [(a,"2028-02-29","10"),(z,"2028-01-31","002.0")] {
                db.compare_set_property(note,"日期",None,Some(date))?;
                db.compare_set_property(note,"優先",None,Some(number))?;
                db.compare_set_property(note,"分類",None,Some("工程"))?;
            }
            let calendar = db.save_presented_collection_view(collection,"日期月曆",CollectionLayout::Table,
                "","",None,Presentation {calendar_key:Some("日期".into()),..Default::default()})?;
            let grouped = db.save_presented_collection_view(collection,"工作分組",CollectionLayout::Board,
                "","",None,Presentation {group_key:Some("分類".into()),sort_key:Some("優先".into()),..Default::default()})?;
            assert_eq!(db.calendar_collection_rows(collection,&calendar,feb)?[&29][0].id,a);
            assert_eq!(db.calendar_collection_rows(collection,&calendar,feb.shifted(-1))?[&31][0].id,z);
            db.set_collection_calendar_month(collection,&calendar,feb)?;
            assert!(db.set_collection_calendar_month(collection,&calendar,feb).is_err());
            (collection,a,z,empty,calendar.id,grouped.id)
        };
        {
            let db = Notebook::open(&path,false)?;
            let saved = db.collection_views(collection)?;
            let calendar = saved.iter().find(|v|v.id==calendar_id).unwrap();
            let grouped = saved.iter().find(|v|v.id==grouped_id).unwrap();
            assert_eq!(calendar.presentation.calendar_month,Some(feb));
            let lanes = db.presented_collection_lanes(collection,Some(grouped))?;
            assert_eq!(lanes[0].1[0].id,empty);
            assert_eq!(lanes[1].1.iter().map(|row|row.id).collect::<Vec<_>>(),[z,a]);
            db.compare_set_property(a,"優先",Some("10"),Some("1"))?;
            assert_eq!(db.presented_collection_lanes(collection,Some(grouped))?[1].1[0].id,a);
            db.compare_set_property(a,"日期",Some("2028-02-29"),Some("2028-03-01"))?;
            assert!(db.calendar_collection_rows(collection,calendar,feb)?.is_empty());
            assert_eq!(db.calendar_collection_rows(collection,calendar,feb.shifted(1))?[&1][0].id,a);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn week_bounds_and_list_persist_with_live_rows() -> Result<()> {
        let path = std::env::temp_dir().join(format!("paperroom-schedule-{}-{}.db",
            std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        let day = |raw| CalendarDay::parse(raw).unwrap();
        let monday = day("2025-12-29");
        let thursday = day("2026-01-01");
        let friday = day("2026-01-02");
        let next = day("2026-01-05");
        assert_eq!(thursday.monday(),monday);
        assert_eq!(monday.shifted(7),Some(next));
        assert_eq!(day("2028-02-28").shifted(1),Some(day("2028-02-29")));
        assert_eq!(day("0001-01-01").shifted(-1),None);
        assert_eq!(day("9999-12-31").shifted(1),None);
        let (collection,first,last,week_id,list_id) = {
            let mut db = Notebook::open(&path,false)?;
            let collection = db.create_collection("排程")?;
            db.compare_set_property_type("日期",None,Some(PropertyType::Date))?;
            let first = db.add(None,"工作 A","")?;
            let last = db.add(None,"工作 B","")?;
            let outside = db.add(None,"工作 C","")?;
            for (note,status,date) in [(first,"進行中",thursday),(last,"完成",friday),(outside,"完成",next)] {
                db.add_to_collection(collection,note,status)?;
                db.compare_set_property(note,"日期",None,Some(&date.iso()))?;
            }
            let week = db.save_presented_collection_view(collection,"週行程",CollectionLayout::Table,
                "","",None,Presentation { calendar_key:Some("日期".into()),calendar_week_day:Some(thursday),
                    ..Default::default() })?;
            assert_eq!(week.presentation.calendar_week_day,Some(monday));
            assert!(db.set_collection_calendar_range(collection,&week,Some(friday),Some(thursday)).is_err());
            db.set_collection_calendar_range(collection,&week,Some(thursday),Some(friday))?;
            assert!(db.set_collection_calendar_range(collection,&week,None,None).is_err());
            db.set_collection_calendar_week(collection,&week,next)?;
            assert!(db.set_collection_calendar_week(collection,&week,next).is_err());
            let list = db.save_presented_collection_view(collection,"進行中清單",CollectionLayout::Table,
                "進行中","工作",None,Presentation {list_mode:true,..Default::default()})?;
            (collection,first,last,week.id,list.id)
        };
        {
            let db = Notebook::open(&path,false)?;
            let views = db.collection_views(collection)?;
            let week = views.iter().find(|view|view.id==week_id).unwrap();
            let list = views.iter().find(|view|view.id==list_id).unwrap();
            assert_eq!(week.presentation.calendar_week_day,Some(next));
            assert_eq!(week.presentation.calendar_from,Some(thursday));
            assert_eq!(week.presentation.calendar_until,Some(friday));
            assert!(db.calendar_dated_rows(collection,week,next,next.shifted(6).unwrap())?.is_empty());
            assert_eq!(db.calendar_dated_rows(collection,week,monday,monday.shifted(6).unwrap())?.into_iter()
                .map(|(date,rows)|(date,rows[0].id)).collect::<Vec<_>>(),[(thursday,first),(friday,last)]);
            assert!(list.presentation.list_mode);
            assert_eq!(db.filtered_collection_rows(collection,Some(list))?[0].0.id,first);
            db.move_collection_note(collection,first,"進行中","完成")?;
            assert!(db.filtered_collection_rows(collection,Some(list))?.is_empty());
            db.compare_set_property(first,"日期",Some("2026-01-01"),Some("2026-01-05"))?;
            assert_eq!(db.calendar_dated_rows(collection,week,monday,next.shifted(6).unwrap())?.len(),1);
            db.set_collection_calendar_range(collection,week,None,None)?;
            let week = db.collection_views(collection)?.into_iter().find(|view|view.id==week_id).unwrap();
            assert_eq!(db.calendar_dated_rows(collection,&week,next,next.shifted(6).unwrap())?[&next][0].id,first);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn collection_checkbox_updates_saved_membership_without_rewriting_legacy_values() -> Result<()> {
        let name = format!("paperroom-checkbox-{}-{}.db", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos());
        let path = std::env::temp_dir().join(name);
        let (collection, checked, pending, legacy, complete, unfinished) = {
            let mut db = Notebook::open(&path, false)?;
            let collection = db.create_collection("工作")?;
            let checked = db.add(None,"已完成","")?;
            let pending = db.add(None,"進行中","")?;
            let legacy = db.add(None,"舊資料","")?;
            for note in [checked, pending, legacy] { db.add_to_collection(collection,note,"工作")?; }
            db.compare_set_property(legacy,"完成",None,Some("舊文字"))?;
            db.compare_set_property_type("完成",None,Some(PropertyType::Checkbox))?;
            db.compare_set_property(checked,"完成",None,Some(" TRUE "))?;
            let filter = |value:&str| Some(PropertyFilter {
                key:"完成".into(), kind:PropertyType::Checkbox, value:value.into()
            });
            let complete = db.save_collection_view(collection,"已勾選",CollectionLayout::Table,"","",filter("true"))?;
            let unfinished = db.save_collection_view(collection,"未勾選",CollectionLayout::Table,"","",filter("false"))?;
            assert_eq!(db.filtered_collection_rows(collection,Some(&complete))?[0].0.id,checked);
            assert!(db.filtered_collection_rows(collection,Some(&unfinished))?.is_empty());
            assert!(db.toggle_collection_checkbox(collection,legacy,"工作","完成",Some("舊文字")).is_err());
            assert!(!db.toggle_collection_checkbox(collection,checked,"工作","完成",Some(" TRUE "))?);
            assert!(db.toggle_collection_checkbox(collection,pending,"工作","完成",None)?);
            assert!(db.toggle_collection_checkbox(collection,pending,"工作","完成",None).is_err());
            (collection,checked,pending,legacy,complete,unfinished)
        };
        {
            let db = Notebook::open(&path, false)?;
            let saved = db.collection_views(collection)?;
            assert!(saved.contains(&complete) && saved.contains(&unfinished));
            assert_eq!(db.filtered_collection_rows(collection,Some(&complete))?[0].0.id,pending);
            assert_eq!(db.filtered_collection_rows(collection,Some(&unfinished))?[0].0.id,checked);
            assert_eq!(db.properties(legacy)?[0].1,"舊文字");
            db.move_collection_note(collection,pending,"工作","其他")?;
            assert!(db.toggle_collection_checkbox(collection,pending,"工作","完成",Some("true")).is_err());
            db.compare_set_property_type("完成",Some(PropertyType::Checkbox),Some(PropertyType::Date))?;
            assert!(db.toggle_collection_checkbox(collection,pending,"其他","完成",Some("true")).is_err());
            assert_eq!(db.filtered_collection_rows(collection,Some(&complete))?[0].0.id,pending);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn collection_view_reopens_and_filters_live_rows() -> Result<()> {
        let name = format!("paperroom-view-{}-{}.db",std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos());
        let path = std::env::temp_dir().join(name);
        let (c, other, saved, first) = {
            let mut db = Notebook::open(&path,false)?;
            let c = db.create_collection("工作")?;
            let other = db.create_collection("其他")?;
            let first = db.add(None,"ÉTE 50%","")?;
            for (id,status) in [(first,"開放"),(db.add(None,"éte 50%","")?,"完成"),(db.add(None,"éte 500","")?,"開放")] {
                db.add_to_collection(c,id,status)?;
            }
            let view = db.create_collection_view(c,"進度",CollectionLayout::Table,"開放","éte 50%")?;
            assert!(db.create_collection_view(c,"進度",CollectionLayout::Board,"","").is_err());
            db.create_collection_view(other,"進度",CollectionLayout::Board,"","")?;
            (c,other,view,first)
        };
        {
            let db = Notebook::open(&path,false)?;
            assert_eq!(db.collection_views(c)?,vec![saved.clone()]);
            assert_eq!(db.filtered_collection_rows(c,Some(&saved))?.iter().map(|r|r.0.id).collect::<Vec<_>>(),[first]);
            db.move_collection_note(c,first,"開放","完成")?;
            assert!(db.filtered_collection_rows(c,Some(&saved))?.is_empty());
            assert!(db.delete_collection_view(other,saved.id).is_err());
            db.delete_collection_view(c,saved.id)?;
            assert_eq!(db.filtered_collection_rows(c,None)?.len(),3);
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
}
