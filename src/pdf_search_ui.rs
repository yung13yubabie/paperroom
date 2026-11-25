use super::*;
use crate::store::{
    BoardCardLibraryFilter, MAX_PDF_SEARCH_QUERY, PdfAnnotationHit, PdfExcerptLibraryFilter,
    PdfExcerptLibraryHit, PdfExcerptLibrarySource,
};
use std::io;

const QUERY: i32 = 101;
const PAGE: i32 = 102;
const MATCHES: i32 = 103;
const DETAILS: i32 = 104;
const STATUS: i32 = 105;
const RUN_SEARCH: i32 = 110;
const OPEN_NOTE: i32 = 111;
const OPEN_CARD: i32 = 112;
const MARK_STAR: i32 = 113;
const UNMARK_STAR: i32 = 114;
const LIBRARY_SOURCE: i32 = 120;
const LIBRARY_SCOPE: i32 = 121;
const LIBRARY_TYPE: i32 = 122;
const LIBRARY_STAR: i32 = 123;
const LIBRARY_TAG: i32 = 124;

#[derive(Clone, Copy)]
enum Action {
    Pdf,
    Source,
}

struct Search<'a> {
    notebook: &'a Notebook,
    hits: Vec<PdfAnnotationHit>,
    selected: Option<(i64, Action)>,
    error: Option<Box<dyn std::error::Error>>,
}

enum LibraryAction {
    OpenPdf(i64),
    OpenCard(i64),
    Place(Vec<i64>),
}

struct ExcerptLibrary<'a> {
    notebook: &'a Notebook,
    board: i64,
    sources: Vec<PdfExcerptLibrarySource>,
    hits: Vec<PdfExcerptLibraryHit>,
    selected: Option<LibraryAction>,
    error: Option<Box<dyn std::error::Error>>,
    changed_stars: bool,
}

impl App {
    pub(in crate::window) unsafe fn board_card_library_ui(&mut self) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let mut state = ExcerptLibrary {
                notebook: &self.notebook, board, sources: self.notebook.pdf_excerpt_library_sources()?,
                hits: Vec::new(), selected: None, error: None, changed_stars: false,
            };
            let template = library_template();
            let module = GetModuleHandleW(null());
            if module.is_null() {
                return Err(io::Error::last_os_error().into());
            }
            let enabled = IsWindowEnabled(self.hwnd) != 0;
            SetLastError(0);
            let outcome = DialogBoxIndirectParamW(
                module, template.as_ptr().cast(), self.hwnd, Some(library_dialog_proc), (&raw mut state) as LPARAM,
            );
            let error = GetLastError();
            if enabled && IsWindow(self.hwnd) != 0 && IsWindowEnabled(self.hwnd) == 0 {
                EnableWindow(self.hwnd, 1);
            }
            let ExcerptLibrary { error: state_error, changed_stars, selected, .. } = state;
            if let Some(error) = state_error {
                return Err(error);
            }
            if outcome == -1 {
                return Err(if error == 0 { "無法開啟卡片資料庫。".into() }
                    else { io::Error::from_raw_os_error(error as i32).into() });
            }
            if changed_stars {
                self.rebuild();
            }
            match selected {
                Some(LibraryAction::OpenPdf(note)) => self.open_pdf_excerpt_source(note)?,
                Some(LibraryAction::OpenCard(note)) => self.open_anchored_note(note, None)?,
                Some(LibraryAction::Place(cards)) => {
                    let added = self.notebook.place_board_library_cards(board, &cards)?;
                    let status = format!("已放入 {added} 張原卡片，已有 {} 張。", cards.len() - added);
                    if let Err(error) = self.refresh_board() {
                        label(self.widgets.status, &format!("{status}目前無法重新整理白板。"));
                        self.show_error(error);
                    } else {
                        label(self.widgets.status, &status);
                    }
                }
                None => {}
            }
            Ok(())
        }
    }

    pub(in crate::window) unsafe fn search_pdf_annotations_ui(&mut self) -> Result<()> {
        unsafe {
            let Some((id, action)) = show_search(self.hwnd, &self.notebook)? else {
                return Ok(());
            };
            // The list may be stale if another window trashed the source,
            // deleted its attachment or removed this annotation meanwhile.
            let hit = self.notebook.pdf_annotation_hit(id)?
                .ok_or("這筆 PDF 標註已移除，或來源筆記已移到垃圾桶。")?;
            self.open_anchored_note(hit.note_id, None)?;
            if matches!(action, Action::Pdf) {
                self.open_pdf_annotation_at(hit.attachment_id, hit.page, id)?;
            }
            Ok(())
        }
    }
}

unsafe extern "system" fn library_dialog_proc(window: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    unsafe {
        let id = wp as u16 as i32;
        if msg == WM_CLOSE || (msg == WM_COMMAND && id == IDCANCEL) {
            EndDialog(window, IDCANCEL as isize);
            return 1;
        }
        if msg == WM_INITDIALOG {
            let state = lp as *mut ExcerptLibrary<'_>;
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
            SendDlgItemMessageW(window, QUERY, EM_SETLIMITTEXT, 360, 0);
            SendDlgItemMessageW(window, LIBRARY_TAG, EM_SETLIMITTEXT, 360, 0);
            SendDlgItemMessageW(window, PAGE, EM_SETLIMITTEXT, 7, 0);
            if let Err(error) = fill_library_filters(window, &(*state).sources) {
                (*state).error = Some(error);
                EndDialog(window, IDCANCEL as isize);
                return 1;
            }
            if let Err(error) = run_library_search(window, &mut *state) {
                reset_library_results(window, &mut *state, &error.to_string());
            }
            return 1;
        }
        if msg != WM_COMMAND || !matches!(id,
            RUN_SEARCH | QUERY | PAGE | LIBRARY_TAG | LIBRARY_SOURCE | LIBRARY_SCOPE | LIBRARY_TYPE |
            LIBRARY_STAR | MATCHES | IDOK | OPEN_NOTE | OPEN_CARD | MARK_STAR | UNMARK_STAR)
        {
            return 0;
        }
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut ExcerptLibrary<'_>;
        let Some(state) = state.as_mut() else { return 0 };
        let notification = (wp >> 16) as u16 as u32;
        match id {
            RUN_SEARCH if notification == BN_CLICKED => {
                if let Err(error) = run_library_search(window, state) {
                    reset_library_results(window, state, &error.to_string());
                }
            }
            QUERY | PAGE | LIBRARY_TAG if notification == EN_CHANGE => {
                reset_library_results(window, state, "條件已修改。請按搜尋。");
            }
            LIBRARY_SOURCE | LIBRARY_SCOPE | LIBRARY_TYPE | LIBRARY_STAR if notification == CBN_SELCHANGE => {
                reset_library_results(window, state, "條件已修改。請按搜尋。");
            }
            MATCHES if notification == LBN_SELCHANGE => {
                refresh_library_detail(window, &state.hits);
            }
            MARK_STAR | UNMARK_STAR if notification == BN_CLICKED => {
                if let Err(error) = apply_library_stars(window, state, id == MARK_STAR) {
                    reset_library_results(window, state, &error.to_string());
                }
            }
            IDOK | OPEN_NOTE | OPEN_CARD if notification == BN_CLICKED => {
                match library_selection(window, &state.hits) {
                    Ok(selected) => {
                        state.selected = match (id, selected.as_slice()) {
                            (OPEN_NOTE, [hit]) if hit.source.is_some() => Some(LibraryAction::OpenPdf(hit.card)),
                            (OPEN_CARD, [hit]) => Some(LibraryAction::OpenCard(hit.card)),
                            (IDOK, hits) if !hits.is_empty() =>
                                Some(LibraryAction::Place(hits.iter().map(|hit| hit.card).collect())),
                            _ => None,
                        };
                        if state.selected.is_some() {
                            EndDialog(window, IDOK as isize);
                        }
                    }
                    Err(error) => reset_library_results(window, state, &error.to_string()),
                }
            }
            _ => return 0,
        }
        1
    }
}

unsafe fn fill_library_filters(window: HWND, sources: &[PdfExcerptLibrarySource]) -> Result<()> {
    unsafe {
        let labels = ["所有來源".to_owned(), "來源無法開啟".to_owned()].into_iter().chain(
            sources.iter().map(|source| format!("附件 #{}｜{}（筆記 #{}）｜{}",
                source.attachment, single_line(&source.title, 32), source.note, single_line(&source.file, 32))),
        );
        for label in labels {
            if SendDlgItemMessageW(window, LIBRARY_SOURCE, CB_ADDSTRING, 0, wide(&label).as_ptr() as LPARAM) < 0 {
                return Err("無法載入 PDF 來源清單。".into());
            }
        }
        for (control, choices) in [
            (LIBRARY_SCOPE, ["所有卡片", "目前白板", "尚未放入目前白板"]),
            (LIBRARY_TYPE, ["全部卡片", "筆記卡", "PDF 摘錄卡"]),
            (LIBRARY_STAR, ["全部", "已標星", "未標星"]),
        ] {
            for label in choices {
                if SendDlgItemMessageW(window, control, CB_ADDSTRING, 0, wide(label).as_ptr() as LPARAM) < 0 {
                    return Err("無法載入卡片篩選條件。".into());
                }
            }
            SendDlgItemMessageW(window, control, CB_SETCURSEL, 0, 0);
        }
        SendDlgItemMessageW(window, LIBRARY_SOURCE, CB_SETCURSEL, 0, 0);
        Ok(())
    }
}

unsafe fn run_library_search(window: HWND, state: &mut ExcerptLibrary<'_>) -> Result<()> {
    unsafe {
        let text = read_field(window, QUERY, 360)?;
        let page = read_field(window, PAGE, 7)?;
        let page = if page.trim().is_empty() { None } else {
            Some(page.trim().parse().map_err(|_| "頁碼請輸入正整數，或保留空白。")?)
        };
        let source = SendDlgItemMessageW(window, LIBRARY_SOURCE, CB_GETCURSEL, 0, 0);
        let attachment = match source {
            0 | 1 => None,
            index if index >= 2 => Some(state.sources.get((index - 2) as usize).ok_or("請重新選取來源。")?.attachment),
            _ => return Err("請選取來源。".into()),
        };
        let on_board = match SendDlgItemMessageW(window, LIBRARY_SCOPE, CB_GETCURSEL, 0, 0) {
            0 => None, 1 => Some(true), 2 => Some(false), _ => return Err("請選取白板範圍。".into()),
        };
        let pdf_only = match SendDlgItemMessageW(window, LIBRARY_TYPE, CB_GETCURSEL, 0, 0) {
            0 => None, 1 => Some(false), 2 => Some(true), _ => return Err("請選取卡片類型。".into()),
        };
        let starred = match SendDlgItemMessageW(window, LIBRARY_STAR, CB_GETCURSEL, 0, 0) {
            0 => None, 1 => Some(true), 2 => Some(false), _ => return Err("請選取標星狀態。".into()),
        };
        let tag = read_field(window, LIBRARY_TAG, 360)?;
        let (hits, truncated) = state.notebook.board_card_library(state.board, BoardCardLibraryFilter {
            pdf: PdfExcerptLibraryFilter { attachment, unavailable_only: source == 1, on_board, page, text: &text },
            pdf_only, starred, tag: &tag,
        })?;
        SendDlgItemMessageW(window, MATCHES, LB_RESETCONTENT, 0, 0);
        for hit in &hits {
            let range = match hit.pages {
                [Some(a), Some(b)] => format!("第 {a}、{b} 頁"),
                [Some(page), None] => format!("第 {page} 頁"),
                _ => "頁碼未保存".to_owned(),
            };
            let place = if hit.on_board { "已在白板" } else { "尚未放入" };
            let source = hit.source.as_ref().map_or_else(|| "來源無法開啟".to_owned(),
                |source| format!("附件 #{}", source.attachment));
            let kind = if hit.is_pdf { format!("{source} · {range}") } else { "筆記卡".to_owned() };
            let star = if hit.starred { "★ " } else { "" };
            let row = format!("{star}卡片 #{}｜{kind}｜{} · {place}", hit.card, single_line(&hit.title, 48));
            if SendDlgItemMessageW(window, MATCHES, LB_ADDSTRING, 0, wide(&row).as_ptr() as LPARAM) < 0 {
                return Err("無法顯示完整的卡片清單。".into());
            }
        }
        state.hits = hits;
        let status = if truncated { "已達顯示或標籤掃描上限。請縮小搜尋範圍。".to_owned() }
            else { format!("找到 {} 張。Ctrl 或 Shift 可多選。", state.hits.len()) };
        set_text(window, STATUS, &status);
        refresh_library_detail(window, &state.hits);
        Ok(())
    }
}

unsafe fn apply_library_stars(window: HWND, state: &mut ExcerptLibrary<'_>, starred: bool) -> Result<()> {
    unsafe {
        let cards: Vec<_> = library_selection(window, &state.hits)?.into_iter().map(|hit| hit.card).collect();
        if cards.is_empty() { return Ok(()) }
        let changed = state.notebook.set_board_library_stars(&cards, starred)?;
        state.changed_stars |= changed != 0;
        run_library_search(window, state)?;
        for (index, hit) in state.hits.iter().enumerate() {
            if cards.contains(&hit.card) {
                SendDlgItemMessageW(window, MATCHES, LB_SETSEL, 1, index as LPARAM);
            }
        }
        refresh_library_detail(window, &state.hits);
        let status = if starred { format!("已標星 {changed} 張卡片。") }
            else { format!("已取消 {changed} 張卡片的標星。") };
        set_text(window, STATUS, &status);
        Ok(())
    }
}

unsafe fn reset_library_results(window: HWND, state: &mut ExcerptLibrary<'_>, status: &str) {
    unsafe {
        SendDlgItemMessageW(window, MATCHES, LB_RESETCONTENT, 0, 0);
        state.hits.clear();
        set_text(window, STATUS, status);
        refresh_library_detail(window, &state.hits);
    }
}

unsafe fn library_selection<'a>(window: HWND, hits: &'a [PdfExcerptLibraryHit]) -> Result<Vec<&'a PdfExcerptLibraryHit>> {
    unsafe {
        let count = SendDlgItemMessageW(window, MATCHES, LB_GETSELCOUNT, 0, 0);
        if count < 0 || count as usize > hits.len() {
            return Err("摘錄卡選取已變更。請重新搜尋。".into());
        }
        if count == 0 { return Ok(Vec::new()) }
        let mut indices = vec![0_i32; count as usize];
        let read = SendDlgItemMessageW(window, MATCHES, LB_GETSELITEMS, count as WPARAM, indices.as_mut_ptr() as LPARAM);
        if read != count { return Err("摘錄卡選取已變更。請重新搜尋。".into()) }
        indices.into_iter().map(|index| hits.get(index as usize).ok_or_else(|| "摘錄卡選取已變更。請重新搜尋。".into())).collect()
    }
}

unsafe fn refresh_library_detail(window: HWND, hits: &[PdfExcerptLibraryHit]) {
    unsafe {
        let selection = library_selection(window, hits);
        let selected = selection.as_deref().unwrap_or_default();
        let detail = match selection.as_deref() {
            Ok([hit]) => library_detail(hit),
            Ok(hits) if !hits.is_empty() => format!("已選取 {} 張卡片。選取單張可預覽內容。", hits.len()),
            Ok(_) => String::new(),
            Err(error) => error.to_string(),
        };
        set_text(window, DETAILS, &detail);
        EnableWindow(GetDlgItem(window, OPEN_NOTE), i32::from(matches!(selected, [hit] if hit.source.is_some())));
        EnableWindow(GetDlgItem(window, OPEN_CARD), i32::from(selected.len() == 1));
        EnableWindow(GetDlgItem(window, IDOK), i32::from(!selected.is_empty()));
        EnableWindow(GetDlgItem(window, MARK_STAR), i32::from(selected.iter().any(|hit| !hit.starred)));
        EnableWindow(GetDlgItem(window, UNMARK_STAR), i32::from(selected.iter().any(|hit| hit.starred)));
    }
}

fn library_detail(hit: &PdfExcerptLibraryHit) -> String {
    use std::fmt::Write;
    let star = if hit.starred { "★ " } else { "" };
    let mut detail = format!("{star}{}（卡片 #{}）\r\n", hit.title, hit.card);
    if let Some(source) = &hit.source {
        let _ = writeln!(detail, "來源筆記　{}（#{}）｜PDF　{}（附件 #{}）\r",
            source.title, source.note, source.file, source.attachment);
    } else if hit.is_pdf {
        detail.push_str("原始 PDF 目前無法開啟。仍可保留或放置原卡片。\r\n");
    }
    for (index, page) in hit.pages.iter().enumerate() {
        if let Some(page) = page {
            let state = if hit.live[index] { "原標註" } else { "保存的摘錄" };
            let _ = write!(detail, "\r\n第 {page} 頁（{state}）\r\n{}\r\n{}\r\n",
                multiline(&hit.quotes[index]), multiline(&hit.comments[index]));
        }
    }
    if hit.quotes.iter().all(String::is_empty) && hit.comments.iter().all(String::is_empty) {
        detail.push_str(&format!("\r\n卡片內容\r\n{}", multiline(&hit.body)));
    }
    detail
}

unsafe fn show_search(owner: HWND, notebook: &Notebook) -> Result<Option<(i64, Action)>> {
    let template = template();
    let mut state = Search { notebook, hits: Vec::new(), selected: None, error: None };
    unsafe {
        let module = GetModuleHandleW(null());
        if module.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let was_enabled = IsWindowEnabled(owner) != 0;
        SetLastError(0);
        let code = DialogBoxIndirectParamW(
            module, template.as_ptr().cast(), owner, Some(dialog_proc), (&raw mut state) as LPARAM,
        );
        let error = GetLastError();
        if was_enabled && IsWindow(owner) != 0 && IsWindowEnabled(owner) == 0 {
            EnableWindow(owner, 1);
        }
        if let Some(error) = state.error {
            return Err(error);
        }
        if code == -1 {
            return Err(if error != 0 {
                io::Error::from_raw_os_error(error as i32).into()
            } else {
                "無法開啟 PDF 標註搜尋。".into()
            });
        }
        Ok(state.selected)
    }
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
            SendDlgItemMessageW(window, QUERY, EM_SETLIMITTEXT, MAX_PDF_SEARCH_QUERY * 2, 0);
            SendDlgItemMessageW(window, PAGE, EM_SETLIMITTEXT, 7, 0);
            update_details(window, None);
            return 1;
        }
        if msg != WM_COMMAND || !matches!(id, RUN_SEARCH | MATCHES | IDOK | OPEN_NOTE) {
            return 0;
        }
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Search<'_>;
        if state.is_null() {
            return 0;
        }
        let notification = (wp >> 16) as u16 as u32;
        match id {
            RUN_SEARCH if notification == BN_CLICKED => {
                if let Err(error) = run_search(window, &mut *state) {
                    SendDlgItemMessageW(window, MATCHES, LB_RESETCONTENT, 0, 0);
                    (*state).hits.clear();
                    update_details(window, None);
                    set_text(window, STATUS, &error.to_string());
                }
            }
            MATCHES if notification == LBN_SELCHANGE => {
                let selected = selection(window, &(*state).hits);
                update_details(window, selected);
            }
            IDOK | OPEN_NOTE | MATCHES
                if (id == MATCHES && notification == LBN_DBLCLK)
                   || (id != MATCHES && notification == BN_CLICKED) =>
            {
                if let Some(hit) = selection(window, &(*state).hits) {
                    let action = if id == OPEN_NOTE { Action::Source } else { Action::Pdf };
                    (*state).selected = Some((hit.id, action));
                    EndDialog(window, IDOK as isize);
                }
            }
            _ => return 0,
        }
        1
    }
}

unsafe fn run_search(window: HWND, state: &mut Search<'_>) -> Result<()> {
    unsafe {
        let query = read_field(window, QUERY, MAX_PDF_SEARCH_QUERY * 2)?;
        let page = read_field(window, PAGE, 7)?;
        let page = if page.trim().is_empty() {
            None
        } else {
            Some(page.trim().parse::<u32>().map_err(|_| "頁碼請輸入正整數，或保留空白。")?)
        };
        let (hits, truncated) = state.notebook.search_pdf_annotations(&query, page)?;
        let list = GetDlgItem(window, MATCHES);
        SendMessageW(list, LB_RESETCONTENT, 0, 0);
        for hit in &hits {
            let name = list_label(hit);
            let item = SendMessageW(list, LB_ADDSTRING, 0, wide(&name).as_ptr() as LPARAM);
            if item < 0 {
                return Err("無法顯示完整的標註搜尋結果。".into());
            }
        }
        let status = if truncated {
            "顯示最近 100 筆。請用文字或頁碼縮小搜尋。".to_owned()
        } else if hits.is_empty() {
            "沒有符合的 PDF 標註。".to_owned()
        } else {
            format!("找到 {} 筆 PDF 標註。", hits.len())
        };
        set_text(window, STATUS, &status);
        state.hits = hits;
        if !state.hits.is_empty() {
            SendMessageW(list, LB_SETCURSEL, 0, 0);
        }
        update_details(window, state.hits.first());
        Ok(())
    }
}

fn single_line(text: &str, limit: usize) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(limit).collect()
}

fn list_label(hit: &PdfAnnotationHit) -> String {
    let title = single_line(&hit.note_title, 40);
    let attachment = single_line(&hit.attachment_name, 40);
    let summary = if hit.quote.is_empty() { &hit.comment } else { &hit.quote };
    format!("{title}（#{}）｜{attachment} · 第 {} 頁｜{}", hit.note_id, hit.page, single_line(summary, 50))
}

unsafe fn selection(window: HWND, hits: &[PdfAnnotationHit]) -> Option<&PdfAnnotationHit> {
    let index = unsafe { SendDlgItemMessageW(window, MATCHES, LB_GETCURSEL, 0, 0) };
    usize::try_from(index).ok().and_then(|index| hits.get(index))
}

unsafe fn update_details(window: HWND, hit: Option<&PdfAnnotationHit>) {
    unsafe {
        let detail = hit.map_or_else(String::new, |hit| {
            format!("來源筆記　{}（#{}）\r\n附件　{} · 第 {} 頁\r\n\r\n引文\r\n{}\r\n\r\n註解\r\n{}",
                hit.note_title, hit.note_id, hit.attachment_name, hit.page,
                multiline(&hit.quote), multiline(&hit.comment))
        });
        set_text(window, DETAILS, &detail);
        for id in [IDOK, OPEN_NOTE] {
            EnableWindow(GetDlgItem(window, id), i32::from(hit.is_some()));
        }
    }
}

fn multiline(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n")
}

unsafe fn set_text(window: HWND, id: i32, text: &str) {
    unsafe { SetDlgItemTextW(window, id, wide(text).as_ptr()); }
}

unsafe fn read_field(window: HWND, id: i32, max_units: usize) -> Result<String> {
    let mut units = vec![0; max_units + 1];
    let read = unsafe { GetDlgItemTextW(window, id, units.as_mut_ptr(), units.len() as i32) };
    Ok(String::from_utf16(&units[..read as usize])?)
}

fn template() -> Vec<u32> {
    let mut words = Vec::new();
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | (DS_SETFONT | DS_CENTER | DS_MODALFRAME) as u32;
    words.extend([style as u16, (style >> 16) as u16, 0, 0, 11, 0, 0, 396, 247, 0, 0]);
    words.extend(wide("搜尋 PDF 標註"));
    words.push(10);
    words.extend(wide("Microsoft JhengHei UI"));
    let text = (0x82, 0, 0);
    let entry = (0x81, WS_TABSTOP | ES_AUTOHSCROLL as u32, WS_EX_CLIENTEDGE);
    let button = (0x80, WS_TABSTOP, 0);
    control(&mut words, [8, 8, 75, 16], -1, text, "搜尋文字");
    control(&mut words, [8, 22, 241, 16], QUERY, entry, "");
    control(&mut words, [257, 8, 52, 16], -1, text, "頁碼");
    control(&mut words, [257, 22, 54, 16], PAGE, entry, "");
    control(&mut words, [321, 22, 67, 16], RUN_SEARCH,
            (0x80, WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 0), "搜尋");
    control(&mut words, [8, 44, 380, 14], STATUS, text, "輸入引文、註解、來源筆記或附件名稱。兩欄均可留空。");
    control(&mut words, [8, 59, 380, 92], MATCHES,
            (0x83, WS_TABSTOP | WS_VSCROLL | (LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32, WS_EX_CLIENTEDGE), "");
    control(&mut words, [8, 156, 380, 62], DETAILS, (0x81, WS_TABSTOP | WS_VSCROLL |
        (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32, WS_EX_CLIENTEDGE), "");
    control(&mut words, [162, 225, 72, 16], OPEN_NOTE, button, "開啟來源筆記");
    control(&mut words, [239, 225, 72, 16], IDOK, button, "開啟 PDF");
    control(&mut words, [316, 225, 72, 16], IDCANCEL, button, "關閉");
    if words.len() % 2 != 0 { words.push(0); }
    words.chunks_exact(2).map(|pair| u32::from(pair[0]) | (u32::from(pair[1]) << 16)).collect()
}

fn library_template() -> Vec<u32> {
    let mut words = Vec::new();
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | (DS_SETFONT | DS_CENTER | DS_MODALFRAME) as u32;
    words.extend([style as u16, (style >> 16) as u16, 0, 0, 24, 0, 0, 430, 296, 0, 0]);
    words.extend(wide("卡片資料庫"));
    words.push(10);
    words.extend(wide("Microsoft JhengHei UI"));
    let text = (0x82, 0, 0);
    let entry = (0x81, WS_TABSTOP | ES_AUTOHSCROLL as u32, WS_EX_CLIENTEDGE);
    let combo = (0x85, WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32, 0);
    let button = (0x80, WS_TABSTOP, 0);
    control(&mut words, [8, 8, 95, 14], -1, text, "來源 PDF");
    control(&mut words, [8, 22, 145, 120], LIBRARY_SOURCE, combo, "");
    control(&mut words, [160, 8, 91, 14], -1, text, "白板範圍");
    control(&mut words, [160, 22, 91, 82], LIBRARY_SCOPE, combo, "");
    control(&mut words, [258, 8, 82, 14], -1, text, "卡片類型");
    control(&mut words, [258, 22, 82, 82], LIBRARY_TYPE, combo, "");
    control(&mut words, [347, 8, 75, 14], -1, text, "標星狀態");
    control(&mut words, [347, 22, 75, 82], LIBRARY_STAR, combo, "");
    control(&mut words, [8, 46, 162, 14], -1, text, "卡名、內文、引文或註解");
    control(&mut words, [8, 60, 163, 16], QUERY, entry, "");
    control(&mut words, [179, 46, 104, 14], -1, text, "完整標籤");
    control(&mut words, [179, 60, 104, 16], LIBRARY_TAG, entry, "");
    control(&mut words, [291, 46, 52, 14], -1, text, "頁碼");
    control(&mut words, [291, 60, 52, 16], PAGE, entry, "");
    control(&mut words, [350, 60, 72, 16], RUN_SEARCH,
        (0x80, WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 0), "搜尋");
    control(&mut words, [8, 82, 414, 14], STATUS, text, "");
    control(&mut words, [8, 98, 414, 88], MATCHES,
        (0x83, WS_TABSTOP | WS_VSCROLL | (LBS_NOTIFY | LBS_EXTENDEDSEL | LBS_NOINTEGRALHEIGHT) as u32,
            WS_EX_CLIENTEDGE), "");
    control(&mut words, [8, 191, 414, 72], DETAILS, (0x81, WS_TABSTOP | WS_VSCROLL |
        (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32, WS_EX_CLIENTEDGE), "");
    control(&mut words, [8, 272, 63, 16], OPEN_CARD, button, "開啟卡片");
    control(&mut words, [75, 272, 69, 16], OPEN_NOTE, button, "開啟原 PDF");
    control(&mut words, [148, 272, 55, 16], MARK_STAR, button, "標星");
    control(&mut words, [208, 272, 64, 16], UNMARK_STAR, button, "取消標星");
    control(&mut words, [277, 272, 89, 16], IDOK, button, "放入目前白板");
    control(&mut words, [372, 272, 50, 16], IDCANCEL, button, "關閉");
    if words.len() % 2 != 0 { words.push(0); }
    words.chunks_exact(2).map(|pair| u32::from(pair[0]) | (u32::from(pair[1]) << 16)).collect()
}

fn control(words: &mut Vec<u16>, rect: [u16; 4], id: i32,
           (class, style, ex_style): (u16, u32, u32), label: &str) {
    if words.len() % 2 != 0 { words.push(0); }
    for value in [style | WS_CHILD | WS_VISIBLE, ex_style] {
        words.extend([value as u16, (value >> 16) as u16]);
    }
    words.extend(rect);
    words.extend([id as u16, 0xffff, class]);
    words.extend(wide(label));
    words.push(0);
}
