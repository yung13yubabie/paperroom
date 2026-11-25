use super::*;
use std::ops::Range;

pub(super) fn editor_projection(body: &str) -> String {
    body.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n")
}

unsafe fn source_selection(editor: HWND) -> Result<Range<usize>> {
    let (mut first, mut last) = (u32::MAX, u32::MAX);
    unsafe { SendMessageW(editor, EM_GETSEL, (&raw mut first) as usize, (&raw mut last) as isize); }
    if first == u32::MAX || last == u32::MAX {
        return Err("無法讀取編輯器的文字選取範圍。".into());
    }
    if first >= last { return Err("請先在編輯區選取要標記的文字。".into()); }
    Ok(first as usize..last as usize)
}

impl App {
    unsafe fn highlight_editor(&self, id: i64) -> Result<(&Note, String)> {
        let note = self.current.as_ref().filter(|note| note.row.id == id && note.row.trash_root.is_none())
            .ok_or("目前已切換筆記。請重新選取要標記的文字。")?;
        if !self.editing || !self.editor_ready || self.extras.board {
            return Err("請先開啟使用中筆記的編輯器。".into());
        }
        let title = unsafe { read_text(self.widgets.title)? };
        let body = unsafe { read_text(self.widgets.body)? };
        if self.dirty || self.failed || title != note.row.title || body != editor_projection(&note.body) {
            return Err("編輯器與目前儲存版本不同。請等修改儲存完成後再執行標記。".into());
        }
        Ok((note, body))
    }

    pub(super) unsafe fn edit_note_highlights(&self, id: i64, clear: bool) -> Result<()> {
        let (note, editor) = unsafe { self.highlight_editor(id)? };
        unsafe {
            if clear {
                let approved = self.notebook.note_highlights(id, &note.body)?;
                if approved.is_empty() {
                    label(self.widgets.status, "本篇沒有螢光標記。");
                    return Ok(());
                }
                if message(self.hwnd, "清除螢光標記", "要清除本篇的所有螢光標記嗎？正文會保留。",
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                    return Ok(());
                }
                self.highlight_editor(id)?;
                let count = self.notebook.clear_note_highlights(id, note.version, &note.body, &approved)?;
                label(self.widgets.status, &format!("已清除 {count} 個螢光標記。"));
            } else {
                let selection = source_selection(self.widgets.body)?;
                let added = self.notebook.toggle_note_highlight(id, note.version, &note.body, &editor, selection)?;
                SetFocus(self.widgets.body);
                label(self.widgets.status, if added {
                    "已加上螢光標記。切換閱讀可查看。"
                } else {
                    "已移除選取範圍的螢光標記。"
                });
            }
        }
        Ok(())
    }

    pub(super) fn render_reading_note(&self, note: &Note, active: bool) -> String {
        use crate::markdown::{preview, preview_marked};
        use std::borrow::Cow;
        if !active { return preview(&note.body); }
        let (mut marks, mut warning) = match self.notebook.note_highlights(note.row.id, &note.body) {
            Ok(marks) => (marks, ""),
            Err(_) => (Vec::new(), "\n\n無法載入螢光標記。請重開這篇筆記。"),
        };
        let mut display = Cow::Borrowed(note.body.as_str());
        if note.body.contains("![[") {
            match self.notebook.project_embeds_marked(note.row.id, &note.body, &marks) {
                Ok((projection, positions)) => {
                    display = Cow::Owned(projection);
                    marks = positions;
                }
                Err(_) => warning = "\n\n無法更新嵌入。請重開這篇筆記。",
            }
        }
        if !warning.is_empty() { display.to_mut().push_str(warning); }
        preview_marked(&display, &marks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    struct CharFormat2W {
        size: u32,
        mask: u32,
        effects: u32,
        height: i32,
        offset: i32,
        text_color: u32,
        charset: u8,
        family: u8,
        face: [u16; 32],
        weight: u16,
        spacing: i16,
        background: u32,
        locale: u32,
        reserved: u32,
        style: i16,
        kerning: u16,
        underline: u8,
        animation: u8,
        revision_author: u8,
        underline_color: u8,
    }

    const _: [(); 116] = [(); size_of::<CharFormat2W>()];

    unsafe fn selection_background(field: HWND, span: Range<usize>) -> Option<u32> {
        const EM_GETCHARFORMAT: u32 = WM_USER + 58;
        const SCF_SELECTION: usize = 1;
        const CFM_BACKCOLOR: u32 = 0x04000000;
        const CFE_AUTOBACKCOLOR: u32 = CFM_BACKCOLOR;
        unsafe {
            SendMessageW(field, EM_SETSEL, span.start, span.end as isize);
            let mut format: CharFormat2W = std::mem::zeroed();
            format.size = size_of::<CharFormat2W>() as u32;
            let result = SendMessageW(field, EM_GETCHARFORMAT, SCF_SELECTION, (&raw mut format) as isize);
            assert_ne!(result, 0, "RichEdit did not return character formatting");
            (format.mask & CFM_BACKCOLOR != 0 && format.effects & CFE_AUTOBACKCOLOR == 0)
                .then_some(format.background)
        }
    }

    #[test]
    fn native_reading_control_only_colors_the_saved_unicode_selection() -> Result<()> {
        unsafe {
            let module = GetModuleHandleW(null());
            let rich_edit = LoadLibraryExW(wide("Msftedit.dll").as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32);
            assert!(!rich_edit.is_null());
            init_tree_control()?;
            let host = CreateWindowExW(0, wide("STATIC").as_ptr(), null(), WS_POPUP, 0, 0, 400, 300,
                null_mut(), null_mut(), module, null());
            assert!(!host.is_null());
            let widgets = Widgets::create(host, module)?;

            let offset = 65_537;
            let long = format!("{}📝", "a".repeat(offset));
            assert!(load_editor_text(widgets.body, &long));
            SendMessageW(widgets.body, EM_SETSEL, offset, (offset + 2) as isize);
            assert_eq!(source_selection(widgets.body)?, offset..offset + 2);

            let manuscript = "前方 📘選詞📝 右邊\r\n\r\n保留原來換行";
            let mut db = Notebook::open(Path::new(":memory:"), false)?;
            let id = db.add(None, "原文", manuscript)?;
            let note = db.note(id)?.unwrap();
            let projected_editor = editor_projection(manuscript);
            assert!(load_editor_text(widgets.body, &projected_editor));
            let text = read_text(widgets.body)?;
            let chosen = "選詞📝";
            let first = text[..text.find(chosen).unwrap()].encode_utf16().count();
            let end = first + chosen.encode_utf16().count();
            SendMessageW(widgets.body, EM_SETSEL, first, end as isize);
            let selected = source_selection(widgets.body)?;
            assert!(db.toggle_note_highlight(id, note.version, &note.body, &text, selected)?);
            let marks = db.note_highlights(id, &note.body)?;
            let marked_source = note.body.find(chosen).unwrap();
            assert_eq!(marks, [marked_source..marked_source + chosen.len()]);
            assert_eq!(db.note(id)?.unwrap().body, manuscript);

            SendMessageW(widgets.preview, EM_SETREADONLY, 0, 0);
            set_rtf(widgets.preview, &crate::markdown::preview_marked(&note.body, &marks));
            SendMessageW(widgets.preview, EM_SETREADONLY, 1, 0);
            let rendered = read_text(widgets.preview)?;
            let rendered_first = rendered[..rendered.find(chosen).unwrap()].encode_utf16().count();
            let rendered_last = rendered_first + chosen.encode_utf16().count();
            let next = rendered[..rendered.find("右").unwrap()].encode_utf16().count();
            assert_eq!(selection_background(widgets.preview, rendered_first..rendered_last), Some(color(255, 232, 131)));
            assert_eq!(selection_background(widgets.preview, 0..1), None);
            assert_eq!(selection_background(widgets.preview, next..next + 1), None);
            assert_eq!(read_text(widgets.body)?, projected_editor);
            assert_ne!(DestroyWindow(host), 0);
            FreeLibrary(rich_edit);
        }
        Ok(())
    }
}
