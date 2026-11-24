use super::*;
use crate::store::typed_properties::PropertyType;

impl App {
    pub(super) unsafe fn typed_property_menu(&self) -> Result<()> {
        unsafe {
            let action = popup(self.hwnd,self.widgets.files,None,&[(1,"設定屬性型別…"),(2,"查看工作區屬性定義…")]);
            if action == 2 {
                let defs = self.notebook.property_definitions()?;
                let mut lines = defs.iter().map(|(key,kind)|format!("{key}　{}",kind.hint())).collect::<Vec<_>>();
                if lines.is_empty() { lines.push("目前所有屬性皆使用文字。".into()); }
                return crate::dialog::show_text(self.hwnd,"工作區屬性定義",&lines.join("\r\n"));
            }
            if action != 1 { return Ok(()); }
            let Some(raw) = crate::dialog::prompt(self.hwnd,"設定工作區屬性型別","屬性名稱","",MAX_KEY_BYTES)? else { return Ok(()) };
            let key = key_checked(&raw)?;
            let expected = self.notebook.property_type(key)?;
            let next = match popup(self.hwnd,self.widgets.files,None,&[(1,"數字"),(2,"核取方塊"),(3,"日期"),(4,"文字（移除型別限制）")]) {
                1 => Some(PropertyType::Number), 2 => Some(PropertyType::Checkbox), 3 => Some(PropertyType::Date),
                4 => None, _ => return Ok(()),
            };
            self.notebook.compare_set_property_type(key,expected,next)?;
            label(self.widgets.status,"已更新工作區屬性型別，原有筆記值已保留。");
            Ok(())
        }
    }
}
