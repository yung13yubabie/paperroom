use super::*;

impl App {
    pub(super) unsafe fn note_history_ui(&mut self, note: i64) -> Result<()> {
        unsafe {
            let current = self.current.as_ref().filter(|current| current.row.id == note
                && current.row.trash_root.is_none() && !self.extras.board)
                .ok_or("請先開啟使用中的筆記。")?;
            let version = current.version;
            let draft = if self.editing {
                if !self.editor_ready {
                    return Err("編輯器尚未就緒，請重新開啟筆記。".into());
                }
                let title = read_text(self.widgets.title)?;
                let body = read_text(self.widgets.body)?;
                let projected = note_highlight_ui::editor_projection(&current.body);
                if self.dirty || self.failed || title != current.row.title || body != projected {
                    let body = if body == projected { current.body.clone() }
                        else { body.replace("\r\n", "\n").replace('\r', "\n") };
                    Some((title, body))
                } else { None }
            } else { None };
            let revisions = self.notebook.note_revisions(note)?;
            if revisions.is_empty() {
                label(self.widgets.status, "這篇筆記尚無可還原的舊版本。");
                return Ok(());
            }
            let labels: Vec<_> = revisions.iter().map(|revision| {
                format!("版本 {}　{}　{}　#{}", revision.version, revision.title, revision.timestamp, revision.id)
            }).collect();
            let items: Vec<_> = labels.iter().enumerate()
                .map(|(index, label)| (index + 1, label.as_str())).collect();
            let action = popup(self.hwnd, self.widgets.more, None, &items);
            let Some(revision) = action.checked_sub(1).and_then(|index| revisions.get(index))
            else { return Ok(()); };
            let preview = self.notebook.preview_note_revision(note, revision.id)?;
            let title = preview.title.as_str();
            let body = preview.body.as_str();
            let (baseline, before_title, before_body) = if let Some((title, body)) = &draft {
                ("目前未存草稿", title.as_str(), body.as_str())
            } else {
                ("目前保存版本", current.row.title.as_str(), current.body.as_str())
            };
            let changes = crate::store::note_history::diff::comparison(before_title, before_body, title, body);
            let identical = before_title == title && before_body == body && !preview.metadata_changed;
            let guidance = if draft.is_some() {
                "這份草稿保留在編輯器。先儲存草稿，再重新開啟預覽即可還原。"
            } else if identical {
                "本版與目前保存內容完全相同。"
            } else if before_body != body {
                "還原會保存完整舊版。目前版本會進入歷史。正文變更會清除本篇的螢光標記。"
            } else {
                "還原會保存完整舊版。目前版本會進入歷史。"
            };
            let report = format!("比較 {baseline} 與版本 {}　{}　#{}\n{guidance}\n\n{}\n\n{changes}",
                revision.version, revision.timestamp, revision.id, preview.metadata);
            if draft.is_some() || identical {
                return crate::dialog::show_text(self.hwnd, "筆記版本差異", &report);
            }
            if !crate::dialog::preview_decision(self.hwnd, "筆記版本差異", &report, "還原此版本", "取消")? {
                return Ok(());
            }
            if !self.notebook.restore_note_revision(note, version, revision.id, Some(preview.data_version))? {
                return Ok(());
            }
            self.current = self.notebook.note(note)?;
            self.present();
            self.rebuild();
            label(self.widgets.status, "已還原選取的筆記版本。");
            Ok(())
        }
    }
}
