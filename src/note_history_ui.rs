use super::*;

impl App {
    pub(super) unsafe fn note_history_ui(&mut self, note: i64) -> Result<()> {
        unsafe {
            let version = self.current.as_ref().filter(|current| current.row.id == note)
                .ok_or("請先開啟筆記。")?.version;
            let revisions = self.notebook.note_revisions(note)?;
            if revisions.is_empty() {
                label(self.widgets.status, "這篇筆記尚無可還原的舊版本。");
                return Ok(());
            }
            let labels: Vec<_> = revisions.iter().map(|revision| {
                format!("{}　版本 {}　{}", revision.timestamp, revision.version, revision.title)
            }).collect();
            let items: Vec<_> = labels.iter().enumerate()
                .map(|(index, label)| (index + 1, label.as_str())).collect();
            let action = popup(self.hwnd, self.widgets.more, None, &items);
            let Some(revision) = action.checked_sub(1).and_then(|index| revisions.get(index))
            else { return Ok(()); };
            let (title, body) = self.notebook.note_revision(note, revision.id)?;
            let mut chars = body.chars();
            let mut excerpt: String = chars.by_ref().take(8000).collect();
            if chars.next().is_some() {
                excerpt.push_str("\r\n\r\n（預覽已截短，還原會寫入完整版本。）");
            }
            crate::dialog::show_text(self.hwnd, "筆記舊版",
                &format!("版本 {}　{}\r\n{}\r\n\r\n{}", revision.version, revision.timestamp, title, excerpt),
            )?;
            if message(self.hwnd, "還原筆記版本", "以選取的舊版取代這篇筆記的標題與正文？目前保存的文字會進入版本記錄。",
                       MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                return Ok(());
            }
            self.notebook.restore_note_revision(note, version, revision.id)?;
            self.current = self.notebook.note(note)?;
            self.present();
            self.rebuild();
            label(self.widgets.status, "已還原選取的筆記版本。");
            Ok(())
        }
    }
}
