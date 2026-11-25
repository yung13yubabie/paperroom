use super::*;
use super::navigation_ui::choose_navigation;

impl App {
    pub(super) unsafe fn duplicate_note_ui(&mut self, id: i64) -> Result<()> {
        unsafe {
            let Some(scope) = choose_navigation(
                self.hwnd,
                self.widgets.more,
                &["只複製本篇", "複製本篇及所有使用中的子頁"],
            ) else {
                return Ok(());
            };
            let plan = self.notebook.prepare_note_duplicate(id, scope == 1)?;
            let suffix = "（副本）";
            let max_base = MAX_TITLE_CHARS - suffix.chars().count();
            let mut suggestion: String = plan.source_title().chars().take(max_base).collect();
            suggestion.push_str(suffix);
            let Some(title) = crate::dialog::prompt(
                self.hwnd,
                "副本名稱",
                "副本會放在原筆記旁邊。請輸入同層尚未使用的名稱。",
                &suggestion,
                MAX_TITLE_CHARS,
            )? else {
                return Ok(());
            };
            let details = format!(
                "將建立 {} 篇筆記，保留正文、屬性和 {} 份附件（{} 位元組）。\r\n\r\n                 紙間識別碼及已保留的舊標題連結仍指向原筆記。同名的未綁定連結可能需要重新指定。集合和白板配置、關聯欄位、PDF 標註、版本及 Vault 等來源身分不會複製。\r\n\r\n                 建立「{}」？",
                plan.pages(),
                plan.attachments(),
                plan.attachment_bytes(),
                title.trim(),
            );
            if message(self.hwnd, "確認複製筆記", &details, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES {
                let copied = self.notebook.duplicate_note(&plan, &title)?;
                self.open_import(copied, &format!("已建立 {} 篇筆記的副本。", plan.pages()));
            }
            Ok(())
        }
    }
}
