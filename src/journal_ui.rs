use super::*;
use crate::store::collection_views::{CalendarDay, Month};
use crate::store::journal::JournalDay;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled};

const PREVIOUS: i32 = 101;
const TODAY: i32 = 102;
const NEXT: i32 = 103;
const PICK_MONTH: i32 = 104;
const MONTH_CAPTION: i32 = 110;
const FIRST_CELL: i32 = 200;
const CELLS: i32 = 42;

struct JournalDialog<'a> {
    notebook: &'a Notebook,
    today: CalendarDay,
    month: Cell<Month>,
    selected: Cell<Option<u16>>,
    failure: RefCell<Option<Box<dyn std::error::Error>>>,
}

impl App {
    pub(super) unsafe fn journal_calendar(&mut self) -> Result<()> {
        let today = CalendarDay::local_today()?;
        let state = JournalDialog {
            notebook: &self.notebook, today, month: Cell::new(today.month()),
            selected: Cell::new(None), failure: RefCell::new(None),
        };
        let template = journal_template();
        unsafe {
            let module = GetModuleHandleW(null());
            if module.is_null() { return Err(std::io::Error::last_os_error().into()); }
            let owner_enabled = IsWindowEnabled(self.hwnd) != 0;
            SetLastError(0);
            let code = DialogBoxIndirectParamW(
                module, template.as_ptr().cast(), self.hwnd, Some(journal_proc),
                (&raw const state) as LPARAM,
            );
            let error = GetLastError();
            if owner_enabled && IsWindow(self.hwnd) != 0 && IsWindowEnabled(self.hwnd) == 0 {
                EnableWindow(self.hwnd, 1);
            }
            if let Some(failure) = state.failure.into_inner() { return Err(failure); }
            if code == -1 {
                return Err(if error == 0 { "無法開啟日誌月曆。".into() }
                    else { std::io::Error::from_raw_os_error(error as i32).into() });
            }
            if code == IDOK as isize && let Some(day) = state.selected.get() {
                let month = state.month.get();
                // Resolve this date again in the existing write transaction. The grid is just a read snapshot.
                let id = self.notebook.open_daily(month.year, month.month, day)?;
                self.open_import(id, &format!("已開啟 {}-{day:02} 的日誌。", month.iso()));
            }
        }
        Ok(())
    }
}

fn day_at(month: Month, cell: i32) -> Option<u16> {
    let monday_offset = (month.sunday_offset() + 6) % 7;
    let day = usize::try_from(cell).ok()?.checked_sub(monday_offset)? + 1;
    (day <= month.days()).then_some(day as u16)
}

impl JournalDialog<'_> {
    unsafe fn show_month(&self, window: HWND, month: Month) -> Result<()> {
        let dates = self.notebook.journal_month(month)?;
        self.month.set(month);
        unsafe {
            label(GetDlgItem(window, MONTH_CAPTION), &format!("{:04} 年 {:02} 月", month.year, month.month));
            EnableWindow(GetDlgItem(window, PREVIOUS), i32::from(month.shifted(-1) != month));
            EnableWindow(GetDlgItem(window, NEXT), i32::from(month.shifted(1) != month));
            for cell in 0..CELLS {
                let button = GetDlgItem(window, FIRST_CELL + cell);
                let Some(day) = day_at(month, cell) else {
                    ShowWindow(button, SW_HIDE);
                    continue;
                };
                let mark = match dates[usize::from(day) - 1] {
                    Some(JournalDay::Active { pending: None }) => "待辦未統計".to_owned(),
                    Some(JournalDay::Active { pending: Some(n) }) if n >= 1000 => "待辦 ≥1,000".to_owned(),
                    Some(JournalDay::Active { pending: Some(n) }) if n > 0 => format!("待辦 {n}"),
                    Some(JournalDay::Active { .. }) => "有日誌".to_owned(),
                    Some(JournalDay::Trashed) => "垃圾桶".to_owned(),
                    None => String::new(),
                };
                let is_today = month == self.today.month() && usize::from(day) == self.today.day();
                let date = if is_today { format!("{day} 今天") } else { day.to_string() };
                let caption = if mark.is_empty() { date } else { format!("{date}\n{mark}") };
                label(button, &caption);
                ShowWindow(button, SW_SHOWNA);
            }
        }
        Ok(())
    }

    unsafe fn calendar_command(&self, window: HWND, command: i32) -> Result<bool> {
        let month = self.month.get();
        let change = match command {
            PREVIOUS => month.shifted(-1),
            TODAY => self.today.month(),
            NEXT => month.shifted(1),
            PICK_MONTH => {
                let Some(value) = (unsafe {
                    crate::dialog::prompt(window, "跳至月份", "輸入 YYYY-MM。", &month.iso(), 7)?
                }) else { return Ok(true) };
                let Some(month) = Month::parse(&value) else {
                    unsafe { crate::dialog::show_text(window, "無法跳至月份", "請輸入有效的 YYYY-MM。")?; }
                    return Ok(true);
                };
                month
            }
            id if (FIRST_CELL..FIRST_CELL + CELLS).contains(&id) => {
                if let Some(day) = day_at(month, id - FIRST_CELL) {
                    self.selected.set(Some(day));
                    unsafe { EndDialog(window, IDOK as isize); }
                }
                return Ok(true);
            }
            _ => return Ok(false),
        };
        unsafe { self.show_month(window, change)?; }
        Ok(true)
    }
}

unsafe extern "system" fn journal_proc(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> isize {
    unsafe {
        if message == WM_INITDIALOG {
            let Some(state) = (lp as *const JournalDialog<'_>).as_ref() else {
                EndDialog(window, IDCANCEL as isize);
                return 1;
            };
            SetLastError(0);
            let opened = if SetWindowLongPtrW(window, GWLP_USERDATA, lp) == 0 && GetLastError() != 0 {
                Err(std::io::Error::last_os_error().into())
            } else {
                state.show_month(window, state.month.get())
            };
            if let Err(error) = opened {
                *state.failure.borrow_mut() = Some(error);
                EndDialog(window, IDCANCEL as isize);
            }
            return 1;
        }
        if message == WM_CLOSE || (message == WM_COMMAND && wp as u16 as i32 == IDCANCEL) {
            EndDialog(window, IDCANCEL as isize);
            return 1;
        }
        if message != WM_COMMAND || (wp >> 16) as u16 != BN_CLICKED as u16 { return 0; }
        let raw = GetWindowLongPtrW(window, GWLP_USERDATA);
        let Some(state) = (raw as *const JournalDialog<'_>).as_ref() else { return 0 };
        match state.calendar_command(window, wp as u16 as i32) {
            Ok(taken) => isize::from(taken),
            Err(error) => {
                *state.failure.borrow_mut() = Some(error);
                EndDialog(window, IDCANCEL as isize);
                1
            }
        }
    }
}

fn journal_template() -> Vec<u32> {
    const BUTTON: u16 = 0x80;
    const STATIC: u16 = 0x82;
    const CENTERED_STATIC: u32 = 0x0001;
    fn item(words: &mut Vec<u16>, id: i32, class: u16, style: u32, rect: [u16; 4], caption: &str) {
        if words.len() % 2 != 0 { words.push(0); }
        let style = style | WS_CHILD | WS_VISIBLE;
        words.extend([style as u16, (style >> 16) as u16, 0, 0]);
        words.extend(rect);
        words.extend([id as u16, 0xffff, class]);
        words.extend(caption.encode_utf16().chain([0, 0]));
    }

    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | (DS_SETFONT | DS_CENTER | DS_MODALFRAME) as u32;
    let mut words = vec![style as u16, (style >> 16) as u16, 0, 0,
        (4 + 1 + 7 + CELLS + 1) as u16, 0, 0, 347, 238, 0, 0];
    words.extend("日誌月曆".encode_utf16().chain([0, 10]));
    words.extend("Microsoft JhengHei UI".encode_utf16().chain([0]));
    for (i, (id, caption)) in [(PREVIOUS, "上一月"), (TODAY, "今天"), (NEXT, "下一月"), (PICK_MONTH, "跳至月份…")].into_iter().enumerate() {
        item(&mut words, id, BUTTON, WS_TABSTOP, [7 + i as u16 * 56, 7, 52, 17], caption);
    }
    item(&mut words, MONTH_CAPTION, STATIC, CENTERED_STATIC, [232, 10, 107, 14], "");
    for (i, caption) in ["一", "二", "三", "四", "五", "六", "日"].into_iter().enumerate() {
        item(&mut words, -1, STATIC, CENTERED_STATIC, [7 + i as u16 * 48, 28, 45, 13], caption);
    }
    for cell in 0..CELLS {
        let x = 7 + (cell as u16 % 7) * 48;
        let y = 42 + (cell as u16 / 7) * 28;
        item(&mut words, FIRST_CELL + cell, BUTTON, WS_TABSTOP | BS_MULTILINE as u32, [x, y, 45, 25], "");
    }
    item(&mut words, IDCANCEL, BUTTON, WS_TABSTOP, [285, 213, 54, 17], "關閉");
    if words.len() % 2 != 0 { words.push(0); }
    words.chunks_exact(2).map(|pair| u32::from(pair[0]) | (u32::from(pair[1]) << 16)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_cells_use_monday_and_ignore_the_surrounding_month() {
        let leap = Month::parse("2024-02").unwrap();
        assert_eq!(day_at(leap, 2), None);
        assert_eq!(day_at(leap, 3), Some(1));
        assert_eq!(day_at(leap, 31), Some(29));
        assert_eq!(day_at(leap, 32), None);
        let six_weeks = Month::parse("2026-08").unwrap();
        assert_eq!(day_at(six_weeks, 5), Some(1));
        assert_eq!(day_at(six_weeks, 35), Some(31));
        assert_eq!(day_at(six_weeks, 36), None);
    }
}
