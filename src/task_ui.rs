use super::*;
use crate::store::tasks::{TASK_LIMIT, TaskScope};

impl App {
    pub(super) unsafe fn task_overview(&mut self) -> Result<()> {
        unsafe {
            let mut scopes = vec![("整個工作區", TaskScope::Workspace)];
            if self.extras.board && self.extras.workspace.is_none() {
                scopes.push(("目前白板", TaskScope::Board(self.board_id())));
            } else if !self.extras.board
                && let Some(note) = self.current.as_ref().filter(|note| note.row.trash_root.is_none())
            {
                scopes.push(("目前筆記", TaskScope::Note(note.row.id)));
            }
            let labels: Vec<_> = scopes.iter().map(|(label, _)| *label).collect();
            let Some(scope) = navigation_ui::choose_navigation(self.hwnd, self.widgets.files, &labels)
                else { return Ok(()); };
            let scope = scopes[scope].1;
            let Some(state) = navigation_ui::choose_navigation(
                self.hwnd, self.widgets.files, &["待完成", "已完成", "全部待辦"],
            ) else { return Ok(()); };
            let done = [Some(false), Some(true), None][state];
            let Some(query) = crate::dialog::prompt(
                self.hwnd, "搜尋待辦", "依待辦首行、筆記標題或 Vault 路徑查找。留白列出符合狀態的待辦。",
                "", 4096,
            )? else { return Ok(()); };
            loop {
                let (hits, truncated) = self.notebook.task_overview(scope, done, &query)?;
                if truncated {
                    label(self.widgets.status, &format!("已達掃描上限或 {TASK_LIMIT} 筆上限。請縮小搜尋範圍。"));
                } else if hits.is_empty() {
                    label(self.widgets.status, "沒有符合條件的待辦。");
                }
                if hits.is_empty() { return Ok(()); }
                let Some(selected) = navigation_ui::choose_navigation_page(
                    self.hwnd, self.widgets.files, hits.len(), |i| &hits[i].label,
                ) else { return Ok(()); };
                let hit = &hits[selected];
                let action = if hit.done { "標為待完成" } else { "標為已完成" };
                match popup(self.hwnd, self.widgets.files, None, &[(1, "開啟待辦原文"), (2, action), (3, "返回待辦清單")]) {
                    1 => {
                        let (version, position) = self.notebook.task_position(hit)?;
                        self.open_import(hit.note, "已開啟待辦來源。");
                        if !self.current.as_ref().is_some_and(|note| note.row.id == hit.note && note.version == version) {
                            return Err("待辦來源已變更，請重新搜尋。".into());
                        }
                        if self.focus_source_selection(position.start, position.end)? {
                            label(self.widgets.status, &format!("待辦原文第 {} 行", position.line));
                        }
                        return Ok(());
                    }
                    2 => {
                        self.notebook.toggle_listed_task(hit)?;
                        if self.current.as_ref().is_some_and(|note| note.row.id == hit.note) {
                            self.current = self.notebook.note(hit.note)?;
                            self.present();
                        }
                        label(self.widgets.status, "待辦狀態已寫回原筆記。");
                    }
                    3 => {}
                    _ => return Ok(()),
                }
            }
        }
    }
}
