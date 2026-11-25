use super::*;
use crate::markdown::note_text_tools::{count_prose, find_next, plan_replacement};

const MAX_QUERY: usize = 512;
const MAX_REPLACEMENT: usize = 2048;

impl App {
    pub(super) unsafe fn find_replace_note_ui(&mut self) -> Result<()> {
        unsafe {
            let action = popup(self.hwnd, self.widgets.more, None, &[
                (1, "尋找下一個…"),
                (2, "預覽全部取代…"),
            ]);
            if action == 0 {
                return Ok(());
            }
            let Some(query) = crate::dialog::prompt(
                self.hwnd, "在這篇筆記尋找", "尋找原文（區分大小寫，不使用正規表示式）",
                &self.extras.note_find_query, MAX_QUERY,
            )? else { return Ok(()); };
            if query.is_empty() {
                return Err("請輸入要尋找的文字。".into());
            }
            self.extras.note_find_query = query.clone();
            if action == 1 {
                self.find_in_note(&query)
            } else {
                self.preview_replace_note(&query)
            }
        }
    }

    unsafe fn find_in_note(&mut self, query: &str) -> Result<()> {
        unsafe {
            let Some(note) = &self.current else { return Ok(()); };
            let mut caret = 0u32;
            let body = if self.editing {
                if !self.editor_ready {
                    return Err("編輯器未正確載入。".into());
                }
                SendMessageW(self.widgets.body, EM_GETSEL, 0, (&mut caret as *mut u32) as LPARAM);
                read_text(self.widgets.body)?
            } else {
                editor_lines(&note.body)
            };
            let Some((start, end)) = find_next(&body, query, caret as usize) else {
                label(self.widgets.status, "這篇筆記找不到完全相符的文字。");
                return Ok(());
            };
            if !self.editing {
                self.toggle_edit();
            }
            if !self.editor_ready {
                return Err("編輯器未正確載入。".into());
            }
            SetFocus(self.widgets.body);
            SendMessageW(self.widgets.body, EM_SETSEL, start, end as LPARAM);
            SendMessageW(self.widgets.body, EM_SCROLLCARET, 0, 0);
            label(self.widgets.status, "已選取相符原文。再尋找一次可前往下一處。");
            Ok(())
        }
    }

    unsafe fn preview_replace_note(&mut self, query: &str) -> Result<()> {
        unsafe {
            let Some(note) = self.current.as_ref().filter(|note| note.row.trash_root.is_none())
                else { return Ok(()); };
            let (id, version, title, source) =
                (note.row.id, note.version, note.row.title.clone(), note.body.clone());
            let editor = self.note_editor_snapshot()?;
            if let Some((body, editor_title)) = &editor
                && (body != &editor_lines(&source) || crate::store::clean_title(editor_title) != title)
            {
                return Err("目前草稿內容與已儲存版本不同。請先保存草稿。".into());
            }
            let Some(replacement) = crate::dialog::prompt_multiline(
                self.hwnd, "取代文字", "輸入新文字。留空會刪除相符文字。", "", MAX_REPLACEMENT,
            )? else { return Ok(()); };
            let plan = plan_replacement(&source, query, &replacement, MAX_NOTE_BYTES)?;
            if plan.matches == 0 {
                label(self.widgets.status, "這篇筆記找不到可取代的文字。");
                return Ok(());
            }
            if plan.body == source {
                label(self.widgets.status, "取代內容與原文相同。");
                return Ok(());
            }
            crate::dialog::show_text(self.hwnd, "全部取代預覽", &plan.preview)?;
            let warning = format!("將在目前筆記取代 {} 處相符文字。要保存這項變更嗎？", plan.matches);
            if message(self.hwnd, "套用全部取代？", &warning,
                MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES
            {
                return Ok(());
            }
            if self.extras.board || self.dirty
                || !self.current.as_ref().is_some_and(|note| note.row.id == id
                    && note.version == version && note.body == source && note.row.title == title)
                || self.note_editor_snapshot()? != editor
            {
                return Err("預覽建立後，這篇筆記或草稿已變更。請重新預覽。".into());
            }
            // The database also compares its stored version. An out-of-process
            // edit or a move to Trash cannot be overwritten by this preview.
            let new_version = self.notebook.save(id, version, &title, &plan.body)?;
            if let Some(note) = &mut self.current {
                note.version = new_version;
                note.body = plan.body;
            }
            self.present();
            if !self.editing || self.editor_ready {
                if self.editing {
                    SetFocus(self.widgets.body);
                }
                label(self.widgets.status, &format!("已取代 {} 處並保存。", plan.matches));
            }
            Ok(())
        }
    }

    unsafe fn note_editor_snapshot(&self) -> Result<Option<(String, String)>> {
        unsafe {
            if !self.editing {
                return Ok(None);
            }
            if !self.editor_ready {
                return Err("編輯器未正確載入。".into());
            }
            let body = editor_lines(&read_text(self.widgets.body)?);
            Ok(Some((body, read_text(self.widgets.title)?)))
        }
    }

    pub(super) unsafe fn note_word_count_ui(&mut self) -> Result<()> {
        unsafe {
            let Some(note) = &self.current else { return Ok(()); };
            let body = self.note_editor_snapshot()?.map_or_else(|| note.body.clone(), |(body, _)| body);
            let stats = count_prose(&body);
            let report = format!(
                "詞數　{}\n正文字符（不含空白）　{}\n\n只計 Markdown 正文。中日韓表意文字與假名逐字計，其他字母和數字連續成詞。程式碼、HTML 標記、Markdown 符號與連結目的網址不計。",
                stats.words, stats.characters,
            );
            crate::dialog::show_text(self.hwnd, "字數統計", &report)
        }
    }
}

fn editor_lines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n")
}
