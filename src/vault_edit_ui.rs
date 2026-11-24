use super::*;

impl App {
    pub(super) unsafe fn edit_vault_markdown(&mut self) -> Result<()> {
        unsafe {
            let Some(path) = file_dialog(self.hwnd, "Markdown\0*.md;*.markdown\0\0", "md", None)? else { return Ok(()); };
            let draft = crate::vault_edit::Draft::load(&path)?;
            let hint = format!("確定後會寫回原檔，並在同一資料夾保留舊版 .bak。\r\n{}",path.display());
            let Some(edited) = crate::dialog::prompt_multiline(self.hwnd,"編輯 Vault Markdown 原檔",&hint,draft.text(),MAX_NOTE_BYTES)? else { return Ok(()); };
            match draft.save(&edited) {
                Ok(Some(backup)) => crate::dialog::show_text(self.hwnd,"已寫回 Markdown 原檔",&format!("舊版復原檔（如須復原，請將其內容複製回原檔）\r\n{}\r\n\r\n原檔\r\n{}",backup.display(),path.display())),
                Ok(None) => Ok(()),
                Err(error) => crate::dialog::show_text(self.hwnd,"可複製未寫入的草稿",&format!("{error}\r\n\r\n編輯稿\r\n{edited}")),
            }
        }
    }
}
