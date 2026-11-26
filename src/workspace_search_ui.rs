use super::*;
use crate::store::navigation::BlockNoteState;
use crate::store::workspace_search::{MAX_QUERY_CHARS, RESULT_LIMIT, SearchHit, SearchPosition};

const KEYWORD: i32 = 101;
const TAG: i32 = 102;
const FOLDER: i32 = 103;
const HITS: i32 = 104;
const CONTEXT: i32 = 105;
const STATUS: i32 = 106;
const PDF_SCOPE: i32 = 107;
const SEARCH_LABEL: i32 = 108;
const OPEN_HIT: i32 = 111;
const INITIAL_STATUS: &str = "輸入關鍵字、標籤或來源資料夾，再按搜尋。";

enum Hit {
    Note(SearchHit),
    Annotation(crate::store::PdfAnnotationHit),
}

struct Search<'a> {
    notebook: &'a Notebook,
    hits: Vec<Hit>,
    chosen: Option<Hit>,
    error: Option<Box<dyn std::error::Error>>,
}

impl App {
    pub(super) unsafe fn workspace_search_ui(&mut self) -> Result<()> {
        let Some(hit) = (unsafe { show_search(self.hwnd, &self.notebook)? }) else { return Ok(()); };
        let hit = match hit {
            Hit::Note(hit) => hit,
            Hit::Annotation(hit) => {
                self.notebook.require_pdf_search_hit(&hit)?;
                unsafe {
                    self.open_anchored_note(hit.note_id,None)?;
                    self.open_pdf_annotation_at(hit.attachment_id,hit.page,hit.id)?;
                }
                return Ok(());
            }
        };
        let id = hit.snapshot.id;
        if !self.notebook.search_hit_is_current(&hit)? {
            return Err("搜尋結果已修改、移到垃圾桶或刪除。請重新搜尋。".into());
        }
        unsafe {
            if self.extras.board || !self.current.as_ref().is_some_and(|note| note.row.id == id) {
                self.open_import(id, "已開啟搜尋結果。");
            }
            if hit.snapshot.state(self.current.as_ref()) != BlockNoteState::Unchanged
                || !self.notebook.search_hit_is_current(&hit)?
            {
                return Err("無法確認此筆記仍與搜尋結果相符。請重新開啟筆記再搜尋。".into());
            }
            match hit.position {
                SearchPosition::Body { start, end, line } => {
                    if self.focus_source_selection(start, end)? {
                        label(self.widgets.status, &format!("全域搜尋：正文第 {line} 行"));
                    }
                }
                SearchPosition::Title { start, end } => {
                    SetFocus(self.widgets.title);
                    SendMessageW(self.widgets.title, EM_SETSEL, start, end as isize);
                    label(self.widgets.status, "全域搜尋：標題命中");
                }
                SearchPosition::Note => label(self.widgets.status, "已開啟符合篩選的筆記。"),
            }
        }
        Ok(())
    }
}

unsafe fn show_search(owner: HWND, notebook: &Notebook) -> Result<Option<Hit>> {
    let mut state = Search { notebook, hits: Vec::new(), chosen: None, error: None };
    unsafe {
        let module = GetModuleHandleW(null());
        if module.is_null() { return Err(std::io::Error::last_os_error().into()); }
        let was_enabled = IsWindowEnabled(owner) != 0;
        SetLastError(0);
        let result = DialogBoxIndirectParamW(
            module, template().as_ptr().cast(), owner, Some(dialog_proc), (&raw mut state) as LPARAM,
        );
        let error = GetLastError();
        if was_enabled && IsWindow(owner) != 0 && IsWindowEnabled(owner) == 0 {
            EnableWindow(owner, 1);
        }
        if let Some(error) = state.error { return Err(error); }
        if result == -1 {
            return Err(if error == 0 { "無法開啟全域搜尋。".into() }
                else { std::io::Error::from_raw_os_error(error as i32).into() });
        }
    }
    Ok(state.chosen)
}

unsafe extern "system" fn dialog_proc(window: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    unsafe {
        let id = wp as u16 as i32;
        if msg == WM_CLOSE || (msg == WM_COMMAND && id == IDCANCEL) {
            EndDialog(window, IDCANCEL as isize);
            return 1;
        }
        if msg == WM_INITDIALOG {
            let state = lp as *mut Search<'_>;
            if state.is_null() { return 0; }
            SetLastError(0);
            if SetWindowLongPtrW(window, GWLP_USERDATA, lp) == 0 && GetLastError() != 0 {
                (*state).error = Some(std::io::Error::last_os_error().into());
                EndDialog(window, IDCANCEL as isize);
                return 1;
            }
            for (id, limit) in [(KEYWORD, MAX_QUERY_CHARS * 2), (TAG, 4096), (FOLDER, 4096)] {
                SendDlgItemMessageW(window, id, EM_SETLIMITTEXT, limit, 0);
            }
            update_context(window, None);
            return 1;
        }
        if msg != WM_COMMAND { return 0; }
        let Some(state) = (GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Search<'_>).as_mut()
        else { return 0; };
        let notice = (wp >> 16) as u16 as u32;
        match id {
            KEYWORD | TAG | FOLDER if notice == EN_CHANGE => {
                clear_results(window, state);
                label(GetDlgItem(window, STATUS), INITIAL_STATUS);
            }
            PDF_SCOPE if notice == BN_CLICKED => {
                let pdf = pdf_scope(window);
                label(GetDlgItem(window, SEARCH_LABEL), if pdf {"PDF 引文、註解、附件名或筆記標題"} else {"標題或正文關鍵字"});
                for control in [TAG,FOLDER] {
                    EnableWindow(GetDlgItem(window,control),i32::from(!pdf));
                }
                clear_results(window,state);
                label(GetDlgItem(window,STATUS),if pdf {
                    "輸入 PDF 引文、註解、附件名或筆記標題，再按搜尋。"
                } else {INITIAL_STATUS});
            }
            IDOK if notice == BN_CLICKED => {
                if let Err(error) = run_search(window, state) {
                    clear_results(window, state);
                    label(GetDlgItem(window, STATUS), &error.to_string());
                }
            }
            HITS if notice == LBN_SELCHANGE => {
                update_context(window, selected(window).and_then(|i| state.hits.get(i)));
            }
            HITS | OPEN_HIT if (id == HITS && notice == LBN_DBLCLK) || (id == OPEN_HIT && notice == BN_CLICKED) => {
                if let Some(i) = selected(window).filter(|i| *i < state.hits.len()) {
                    state.chosen = Some(state.hits.remove(i));
                    EndDialog(window, IDOK as isize);
                }
            }
            _ => return 0,
        }
        1
    }
}

unsafe fn clear_results(window: HWND, state: &mut Search<'_>) {
    unsafe {
        SendDlgItemMessageW(window, HITS, LB_RESETCONTENT, 0, 0);
        state.hits.clear();
        update_context(window, None);
    }
}

unsafe fn pdf_scope(window: HWND) -> bool {
    unsafe { SendDlgItemMessageW(window, PDF_SCOPE, BM_GETCHECK, 0, 0) == 1 }
}

unsafe fn run_search(window: HWND, state: &mut Search<'_>) -> Result<()> {
    unsafe {
        let keyword = read_text(GetDlgItem(window, KEYWORD))?;
        let tag = read_text(GetDlgItem(window, TAG))?;
        let folder = read_text(GetDlgItem(window, FOLDER))?;
        let pdf=pdf_scope(window);
        let (hits,limited): (Vec<Hit>,bool) = if pdf {
            if keyword.trim().is_empty() {return Err("請輸入 PDF 標註搜尋文字。".into());}
            let (hits,limited)=state.notebook.search_pdf_annotations(&keyword,None)?;
            (hits.into_iter().map(Hit::Annotation).collect(),limited)
        } else {
            let result=state.notebook.search_workspace(&keyword,&tag,&folder)?;
            (result.hits.into_iter().map(Hit::Note).collect(),result.limited)
        };
        clear_results(window, state);
        for hit in &hits {
            let text=match hit {
                Hit::Note(hit)=>{
                    let path = hit.path.as_ref().map_or_else(String::new, |path| format!("　｜ {path}"));
                    format!("{}　〔#{}〕{path}", hit.title, hit.snapshot.id)
                }
                Hit::Annotation(hit)=> format!("{}　〔附件 #{}／標註 #{}〕　｜ 第 {} 頁　｜ {}",
                    hit.attachment_name,hit.attachment_id,hit.id,hit.page,hit.note_title),
            };
            if SendDlgItemMessageW(window, HITS, LB_ADDSTRING, 0, wide(&text).as_ptr() as LPARAM) < 0 {
                return Err("無法顯示搜尋結果。".into());
            }
        }
        let count = hits.len();
        let status = if limited {
            format!("已達搜尋上限，顯示 {count} 筆（最多 {RESULT_LIMIT} 筆）。請縮小範圍。")
        } else if count == 0 {
            if pdf {"沒有符合條件的 PDF 標註。"} else {"沒有符合條件的筆記。"}.to_owned()
        } else if pdf {
            format!("找到 {count} 筆 PDF 標註。")
        } else {
            format!("找到 {count} 筆筆記。")
        };
        label(GetDlgItem(window, STATUS), &status);
        state.hits = hits;
        if !state.hits.is_empty() { SendDlgItemMessageW(window, HITS, LB_SETCURSEL, 0, 0); }
        update_context(window, state.hits.first());
        Ok(())
    }
}

unsafe fn selected(window: HWND) -> Option<usize> {
    usize::try_from(unsafe { SendDlgItemMessageW(window, HITS, LB_GETCURSEL, 0, 0) }).ok()
}

unsafe fn update_context(window: HWND, hit: Option<&Hit>) {
    unsafe {
        let context = hit.map_or_else(String::new, |hit| {
            let hit=match hit {
                Hit::Annotation(hit)=>return format!("{}　｜ {}　第 {} 頁\r\n引文：{}\r\n註解：{}",
                    hit.note_title,hit.attachment_name,hit.page,hit.quote,hit.comment),
                Hit::Note(hit)=>hit,
            };
            let source = hit.path.as_deref().unwrap_or("工作區筆記");
            let place = match hit.position {
                SearchPosition::Body { line, .. } => format!("正文第 {line} 行"),
                SearchPosition::Title { .. } => "標題".into(),
                SearchPosition::Note => "正文開頭".into(),
            };
            format!("{}\r\n{}　｜　{place}\r\n\r\n{}", hit.title, source, hit.context)
        });
        label(GetDlgItem(window, CONTEXT), &context);
        EnableWindow(GetDlgItem(window, OPEN_HIT), i32::from(hit.is_some()));
    }
}

fn template() -> Vec<u32> {
    let mut words = Vec::new();
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | (DS_SETFONT | DS_CENTER | DS_MODALFRAME) as u32;
    words.extend([style as u16, (style >> 16) as u16, 0, 0, 13, 0, 0, 414, 240, 0, 0]);
    words.extend(wide("全域搜尋筆記"));
    words.push(10);
    words.extend(wide("Microsoft JhengHei UI"));
    let entry = WS_TABSTOP | ES_AUTOHSCROLL as u32;
    let button = WS_TABSTOP;
    use crate::dialog::control;
    control(&mut words, [8, 8, 188, 14], SEARCH_LABEL, 0x82, 0, 0, "標題或正文關鍵字");
    control(&mut words, [8, 22, 191, 16], KEYWORD, 0x81, entry, WS_EX_CLIENTEDGE, "");
    control(&mut words, [206, 8, 200, 14], -1, 0x82, 0, 0, "標籤（#拍攝 含子層，=#拍攝 完全相符）");
    control(&mut words, [206, 22, 200, 16], TAG, 0x81, entry, WS_EX_CLIENTEDGE, "");
    control(&mut words, [8, 45, 300, 14], -1, 0x82, 0, 0, "Vault 來源資料夾（選填，如 製片/分鏡）");
    control(&mut words, [8, 59, 321, 16], FOLDER, 0x81, entry, WS_EX_CLIENTEDGE, "");
    control(&mut words, [337, 59, 69, 16], IDOK, 0x80, button | BS_DEFPUSHBUTTON as u32, 0, "搜尋");
    control(&mut words, [8, 81, 398, 14], STATUS, 0x82, 0, 0, INITIAL_STATUS);
    control(&mut words, [8, 96, 398, 66], HITS, 0x83,
        WS_TABSTOP | WS_VSCROLL | (LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32, WS_EX_CLIENTEDGE, "");
    control(&mut words, [8, 167, 398, 45], CONTEXT, 0x81,
        WS_TABSTOP | WS_VSCROLL | (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32, WS_EX_CLIENTEDGE, "");
    control(&mut words, [8, 217, 172, 16], PDF_SCOPE, 0x80, WS_TABSTOP | BS_AUTOCHECKBOX as u32, 0, "搜尋 PDF 標註");
    control(&mut words, [262, 217, 69, 16], OPEN_HIT, 0x80, button, 0, "開啟命中");
    control(&mut words, [337, 217, 69, 16], IDCANCEL, 0x80, button, 0, "關閉");
    if words.len() % 2 != 0 { words.push(0); }
    words.chunks_exact(2).map(|pair| u32::from(pair[0]) | (u32::from(pair[1]) << 16)).collect()
}
