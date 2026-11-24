use crate::store::Result;
use std::io;
use std::ptr::null;
use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, SetLastError, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{EM_SETLIMITTEXT, EM_SETSEL};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const EDIT: i32 = 101;
const HINT: i32 = 102;
const BUTTON_CLASS: u16 = 0x80;
const EDIT_CLASS: u16 = 0x81;
const STATIC_CLASS: u16 = 0x82;

enum Content<'a> {
    Prompt { hint: &'a str, max_chars: usize, multiline: bool },
    ReadOnly,
    JumpPreview,
}

struct Dialog {
    hint: Vec<u16>,
    initial: Vec<u16>,
    max_chars: Option<usize>,
    original_multiline: Option<String>,
    text: Option<String>,
    error: Option<Box<dyn std::error::Error>>,
}

/// The owner must be null or a live window on the calling UI thread.
pub unsafe fn prompt(
    owner: HWND,
    caption: &str,
    hint: &str,
    initial: &str,
    max_chars: usize,
) -> Result<Option<String>> {
    unsafe { prompt_with_mode(owner, caption, hint, initial, max_chars, false) }
}

/// Enter inserts a newline. Unchanged text keeps its original line endings,
/// and changed text uses LF. The owner must belong to the calling UI thread.
pub unsafe fn prompt_multiline(
    owner: HWND,
    caption: &str,
    hint: &str,
    initial: &str,
    max_chars: usize,
) -> Result<Option<String>> {
    unsafe { prompt_with_mode(owner, caption, hint, initial, max_chars, true) }
}

unsafe fn prompt_with_mode(
    owner: HWND,
    caption: &str,
    hint: &str,
    initial: &str,
    max_chars: usize,
    multiline: bool,
) -> Result<Option<String>> {
    if max_chars == 0 || max_chars > (i32::MAX as usize - 1) / 2 {
        return Err("輸入長度上限無效。".into());
    }
    if initial.chars().count() > max_chars {
        return Err("預設文字超過輸入長度上限。".into());
    }
    unsafe { run(owner, caption, initial, Content::Prompt { hint, max_chars, multiline }).map(|(_, text)| text) }
}

/// The owner must be null or a live window on the calling UI thread.
pub unsafe fn show_text(owner: HWND, caption: &str, body: &str) -> Result<()> {
    unsafe { run(owner, caption, body, Content::ReadOnly).map(|_| ()) }
}

/// Show read-only text. Closing the dialog or accepting its default stays on the current note.
pub unsafe fn preview_with_jump(owner: HWND, caption: &str, body: &str) -> Result<bool> {
    unsafe { run(owner, caption, body, Content::JumpPreview).map(|(jump, _)| jump) }
}

fn terminated(text: &str) -> Result<Vec<u16>> {
    if text.contains('\0') {
        return Err("對話框文字不能包含 NUL 字元。".into());
    }
    Ok(text.encode_utf16().chain(Some(0)).collect())
}

fn logical_lines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

unsafe fn run(owner: HWND, caption: &str, initial: &str, content: Content<'_>) -> Result<(bool, Option<String>)> {
    let preview = matches!(&content, Content::JumpPreview);
    let (hint, max_chars, multiline) = match content {
        Content::Prompt { hint, max_chars, multiline } => (hint, Some(max_chars), multiline),
        Content::ReadOnly | Content::JumpPreview => ("", None, true),
    };
    let initial_display = if multiline {
        terminated(&logical_lines(initial).replace('\n', "\r\n"))?
    } else {
        terminated(initial)?
    };
    let mut state = Dialog {
        hint: terminated(hint)?,
        initial: initial_display,
        max_chars,
        original_multiline: (multiline && max_chars.is_some()).then(|| initial.to_owned()),
        text: None,
        error: None,
    };
    let template = template(caption, max_chars.is_some(), multiline, preview)?;
    unsafe {
        if !owner.is_null() && IsWindow(owner) == 0 {
            return Err("對話框的主視窗已關閉。".into());
        }
        let module = GetModuleHandleW(null());
        if module.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let owner_was_enabled = !owner.is_null() && IsWindowEnabled(owner) != 0;
        SetLastError(0);
        // The Win32 dialog manager handles owner modality, keyboard navigation
        // and forwarding WM_QUIT to the calling message loop.
        let code = DialogBoxIndirectParamW(
            module,
            template.as_ptr().cast(),
            owner,
            Some(dialog_proc),
            (&raw mut state) as LPARAM,
        );
        let error = GetLastError();
        // The quit path can return without the usual EndDialog owner cleanup.
        if owner_was_enabled && IsWindow(owner) != 0 && IsWindowEnabled(owner) == 0 {
            EnableWindow(owner, 1);
        }
        if let Some(error) = state.error {
            return Err(error);
        }
        match code {
            code if code == IDOK as isize => Ok((true, state.text)),
            code if code == IDCANCEL as isize => Ok((false, None)),
            // A quit message can leave the dialog with no result from EndDialog.
            0 if error == 0 => Ok((false, None)),
            _ if error != 0 => Err(io::Error::from_raw_os_error(error as i32).into()),
            _ => Err("無法開啟或完成對話框。".into()),
        }
    }
}

unsafe extern "system" fn dialog_proc(window: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    unsafe {
        if msg == WM_INITDIALOG {
            let state = lp as *mut Dialog;
            if state.is_null() {
                EndDialog(window, IDCANCEL as isize);
                return 1;
            }
            SetLastError(0);
            if SetWindowLongPtrW(window, GWLP_USERDATA, lp) == 0 && GetLastError() != 0 {
                (*state).error = Some(io::Error::last_os_error().into());
                EndDialog(window, IDCANCEL as isize);
                return 1;
            }
            let field = GetDlgItem(window, EDIT);
            let max_chars = (*state).max_chars;
            if field.is_null()
                || (max_chars.is_some() && SetDlgItemTextW(window, HINT, (*state).hint.as_ptr()) == 0)
                || SetWindowTextW(field, (*state).initial.as_ptr()) == 0
            {
                (*state).error = Some("無法初始化對話框的文字欄位。".into());
                EndDialog(window, IDCANCEL as isize);
                return 1;
            }
            if let Some(max_chars) = max_chars {
                SendMessageW(field, EM_SETLIMITTEXT, max_chars * 2, 0);
                SendMessageW(field, EM_SETSEL, 0, -1);
            }
            // EDIT is the first tab stop, so the dialog manager focuses it.
            return 1;
        }

        if msg == WM_CLOSE || (msg == WM_COMMAND && wp as u16 as i32 == IDCANCEL) {
            EndDialog(window, IDCANCEL as isize);
            return 1;
        }
        if msg == WM_COMMAND && wp as u16 as i32 == IDOK {
            let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Dialog;
            if state.is_null() {
                return 0;
            }
            if let Some(max_chars) = (*state).max_chars {
                let field = GetDlgItem(window, EDIT);
                let text = read_edit(field).map(|text| {
                    if let Some(original) = &(*state).original_multiline {
                        let logical = logical_lines(&text);
                        if logical == logical_lines(original) { original.clone() } else { logical }
                    } else {
                        text
                    }
                });
                match text {
                    Ok(text) if text.chars().count() <= max_chars => (*state).text = Some(text),
                    Ok(_) => {
                        let message = format!("文字最多可輸入 {max_chars} 個字元。");
                        let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
                        MessageBoxW(window, message.as_ptr(), null(), MB_OK | MB_ICONWARNING);
                        return 1;
                    }
                    Err(error) => (*state).error = Some(error),
                }
            }
            EndDialog(window, IDOK as isize);
            return 1;
        }
        0
    }
}

unsafe fn read_edit(field: HWND) -> Result<String> {
    unsafe {
        if field.is_null() {
            return Err("找不到對話框的文字欄位。".into());
        }
        SetLastError(0);
        let length = GetWindowTextLengthW(field);
        if length == 0 && GetLastError() != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let capacity = length.checked_add(1).ok_or("對話框文字過長。")?;
        let mut text = vec![0u16; capacity as usize];
        SetLastError(0);
        let read = GetWindowTextW(field, text.as_mut_ptr(), capacity);
        if read != length {
            return Err("無法讀取完整的對話框文字。".into());
        }
        if read == 0 && GetLastError() != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(String::from_utf16(&text[..read as usize])?)
    }
}

fn template(caption: &str, editable: bool, multiline: bool, preview: bool) -> Result<Vec<u32>> {
    let mut words = Vec::<u16>::new();
    let height = if multiline { 175 } else { 95 };
    let count = if editable { 4 } else if preview { 3 } else { 2 };
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | (DS_SETFONT | DS_CENTER | DS_MODALFRAME) as u32;
    words.extend([(style & 0xffff) as u16, (style >> 16) as u16, 0, 0]);
    words.extend([count, 0, 0, 275, height, 0, 0]);
    words.extend(terminated(caption)?);
    words.push(10);
    words.extend(terminated("Microsoft JhengHei UI")?);

    if editable {
        control(&mut words, [8, 7, 259, 26], HINT, STATIC_CLASS, 0, 0, "");
        let (box_height, edit_style) = if multiline {
            (103, WS_VSCROLL | (ES_MULTILINE | ES_WANTRETURN | ES_AUTOVSCROLL) as u32)
        } else {
            (16, ES_AUTOHSCROLL as u32)
        };
        control(
            &mut words, [8, 37, 259, box_height], EDIT, EDIT_CLASS,
            WS_TABSTOP | edit_style, WS_EX_CLIENTEDGE, "",
        );
        let button_top = height - 26;
        control(&mut words, [151, button_top, 55, 16], IDOK, BUTTON_CLASS,
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 0, "確定");
        control(&mut words, [212, button_top, 55, 16], IDCANCEL, BUTTON_CLASS, WS_TABSTOP, 0, "取消");
    } else {
        control(&mut words, [8, 8, 259, 136], EDIT, EDIT_CLASS,
            WS_TABSTOP | WS_VSCROLL | (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32,
            WS_EX_CLIENTEDGE, "");
        if preview {
            control(&mut words, [133, 151, 64, 16], IDOK, BUTTON_CLASS, WS_TABSTOP, 0, "前往區塊");
            control(&mut words, [203, 151, 64, 16], IDCANCEL, BUTTON_CLASS,
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 0, "留在原處");
        } else {
            control(&mut words, [212, 151, 55, 16], IDOK, BUTTON_CLASS,
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 0, "關閉");
        }
    }
    if words.len() % 2 != 0 {
        words.push(0);
    }
    // Windows templates and each item begin on DWORD boundaries.
    Ok(words.chunks_exact(2).map(|pair| u32::from(pair[0]) | (u32::from(pair[1]) << 16)).collect())
}

fn control(
    words: &mut Vec<u16>,
    rect: [u16; 4],
    id: i32,
    class: u16,
    style: u32,
    ex_style: u32,
    caption: &str,
) {
    if words.len() % 2 != 0 {
        words.push(0);
    }
    let style = style | WS_VISIBLE | WS_CHILD;
    for value in [style, ex_style] {
        words.extend([value as u16, (value >> 16) as u16]);
    }
    words.extend(rect);
    words.extend([id as u16, 0xffff, class]);
    words.extend(caption.encode_utf16().chain(Some(0)));
    words.push(0); // No creation data.
}
