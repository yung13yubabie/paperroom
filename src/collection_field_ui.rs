use super::*;
use crate::store::CollectionEntry;
use crate::store::collection_field_rules::{CollectionFieldRule, CollectionRuleInput, RuleValue, validate_rule_input};

impl App {
    pub(super) unsafe fn collection_rule_menu(&self, collection: i64) -> Result<()> {
        let fields = self.notebook.property_definitions()?;
        if fields.is_empty() { return Err("請先從檔案選單設定數字、核取方塊或日期欄位。".into()); }
        let rules = self.notebook.collection_field_rules(collection)?;
        let owner = self.hwnd;
        let anchor = self.widgets.links;
        unsafe {
            let Some(index) = choose(owner, anchor, &fields, |(key,kind)| {
                let rule = rules.iter().find(|rule| rule.key == *key);
                let status = if rule.is_some_and(|rule| rule.required) { "必填" } else { "選填" };
                let default = rule.and_then(|rule| rule.default.as_deref()).unwrap_or("無");
                let short = default.chars().take(40).collect::<String>();
                format!("{key}　{status}　預設：{short}　{}", kind.hint())
            }) else { return Ok(()); };
            let (key, kind) = &fields[index];
            let previous = rules.iter().find(|rule| rule.key == *key);
            let required = match popup(owner, anchor, None, &[(1,"選填"),(2,"必填"),(3,"移除規則")]) {
                1 => Some(false), 2 => Some(true), 3 => None, _ => return Ok(()),
            };
            let mut rule = CollectionFieldRule {
                key: key.clone(), kind: *kind, required: required.unwrap_or(false), default: None,
            };
            if required.is_some() {
                let hint = format!("{key}　{}。留白表示沒有預設值。", kind.hint());
                let mut draft = previous.and_then(|rule| rule.default.clone()).unwrap_or_default();
                loop {
                    let Some(value) = crate::dialog::prompt(owner,"欄位預設值",&hint,&draft,MAX_VALUE_BYTES)?
                    else { return Ok(()); };
                    draft = value;
                    // Defaults may be absent even when the field itself is required.
                    if !draft.trim().is_empty() {
                        if let Err(error) = validate_rule_input(&rule, &draft) {
                            crate::dialog::show_text(owner,"欄位預設值無效",&error.to_string())?;
                            continue;
                        }
                        rule.default = Some(draft);
                    }
                    break;
                }
            }
            if let Err(error) = self.notebook.save_collection_field_rule(collection, &rule, previous) {
                return crate::dialog::show_text(owner,"欄位規則未儲存",&format!(
                    "{error}\r\n\r\n{key}\r\n預設值：{}", rule.default.as_deref().unwrap_or("")
                ));
            }
            label(self.widgets.status,"已儲存集合欄位規則。");
        }
        Ok(())
    }

    pub(super) unsafe fn add_collection_row_ui(
        &mut self, collection: i64, title: String, existing: Option<&Row>,
    ) -> Result<()> {
        let owner = self.hwnd;
        unsafe {
            let note_title = if let Some(row) = existing {
                row.title.clone()
            } else {
                let Some(text) = crate::dialog::prompt(owner,"新增資料列","輸入新筆記標題。","",MAX_TITLE_CHARS)?
                else { return Ok(()); };
                text
            };
            let Some(status) = crate::dialog::prompt(owner,"加入集合","狀態欄名稱","待處理",256)?
            else { return Ok(()); };
            let rules = self.notebook.collection_field_rules(collection)?;
            let properties = existing.map(|row| self.notebook.properties(row.id)).transpose()?.unwrap_or_default();
            let mut values = Vec::with_capacity(rules.len());
            for rule in &rules {
                if let Some((_, value)) = properties.iter().find(|(key, _)| key == &rule.key) {
                    values.push(RuleValue::Existing(value.clone()));
                    continue;
                }
                let presence = if rule.required { "必填" } else { "選填，留白不設定" };
                let hint = format!("{presence}。{}", rule.kind.hint());
                let mut draft = rule.default.clone().unwrap_or_default();
                loop {
                    let Some(value) = crate::dialog::prompt(owner,&rule.key,&hint,&draft,MAX_VALUE_BYTES)?
                    else { return Ok(()); };
                    draft = value;
                    match validate_rule_input(rule, &draft) {
                        Ok(()) => { values.push(RuleValue::Input(draft)); break; }
                        Err(error) => crate::dialog::show_text(owner,"欄位值無效",&error.to_string())?,
                    }
                }
            }
            let entry = existing.map_or(CollectionEntry::New(&note_title), |row| CollectionEntry::Existing(row.id));
            let form = CollectionRuleInput { rules: &rules, values: &values };
            if let Err(error) = self.notebook.add_collection_entry(collection, entry, &status, Some(form)) {
                let mut draft = format!("{error}\r\n\r\n{note_title}\r\n狀態：{status}");
                for (rule, value) in rules.iter().zip(&values) {
                    if let RuleValue::Input(value) = value { draft.push_str(&format!("\r\n{}：{value}", rule.key)); }
                }
                return crate::dialog::show_text(owner,"資料列未儲存（可複製）",&draft);
            }
            self.open_kanban(collection,title)?;
            label(self.widgets.status, if existing.is_some() { "已將筆記加入集合。" } else { "已新增集合資料列。" });
        }
        Ok(())
    }
}
