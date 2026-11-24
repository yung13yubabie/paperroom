use super::*;
use crate::store::zotero::ZoteroImport;

const JSON_FILTER: &str = "Zotero CSL JSON\0*.json\0\0";

impl App {
    pub(super) unsafe fn import_zotero_csl_ui(&mut self) -> Result<()> {
        unsafe {
            let Some(path) = file_dialog(self.hwnd, JSON_FILTER, "json", None)? else {
                return Ok(());
            };
            let plan = ZoteroImport::read(&path)?;
            let count = plan.count();
            crate::dialog::show_text(self.hwnd, "Zotero CSL JSON 匯入預覽", &plan.preview())?;
            let confirmation = format!("建立新的 Zotero 書目批次並匯入 {count} 筆書目？");
            if message(self.hwnd, "確認匯入 Zotero 書目", &confirmation, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                return Ok(());
            }
            let root = self.notebook.import_zotero_csl(plan)?;
            self.open_import(root, &format!("已匯入 {count} 筆 Zotero CSL JSON 書目。"));
            Ok(())
        }
    }

    pub(super) unsafe fn export_zotero_csl_ui(&mut self) -> Result<()> {
        unsafe {
            let current = self.current.as_ref().filter(|note| note.row.trash_root.is_none() && !self.extras.board)
                .ok_or("請先開啟 Zotero 匯入批次的根筆記。")?.row.id;
            let attachment = self.notebook.zotero_original_attachment(current)?;
            let Some(path) = file_dialog(self.hwnd, JSON_FILTER, "json", Some("zotero-original.csl.json"))? else {
                return Ok(());
            };
            self.notebook.save_attachment_copy(attachment, &path)?;
            label(self.widgets.status, "已匯出保留原始位元組的 Zotero CSL JSON。");
            Ok(())
        }
    }
}
