use crate::archive;
use crate::store::{MAX_NOTE_BYTES, MAX_TITLE_CHARS, Note, Notebook, Result, Row, View};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::*;
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const SEARCH: u16 = 101;
const TREE: u16 = 102;
const TITLE: u16 = 103;
const BODY: u16 = 104;
const ALL: u16 = 105;
const PINNED: u16 = 106;
const TRASH: u16 = 107;
const NEW: u16 = 108;
const FILES: u16 = 109;
const EDIT: u16 = 110;
const CHILD: u16 = 111;
const BLOCK: u16 = 112;
const MORE: u16 = 113;
const BRAND: u16 = 114;
const BREADCRUMB: u16 = 115;
const STATUS: u16 = 116;
const EMPTY: u16 = 117;
const SIDE_STATUS: u16 = 118;
const BOARD: u16 = 119;
const LINKS: u16 = 120;
const PREVIEW: u16 = 121;
const WM_OPEN_TREE_NOTE: u32 = WM_APP + 1;
const SAVE_TIMER: usize = 1;

thread_local! {
    static REPAINT_PENDING: Cell<bool> = const { Cell::new(false) };
}

const EM_STREAMIN: u32 = WM_USER + 73;
const EM_SETBKGNDCOLOR: u32 = WM_USER + 67;
const EM_EXLIMITTEXT: u32 = WM_USER + 53;
const SF_RTF: usize = 2;
const SS_LEFTNOWORDWRAP: u32 = 0x0000_000c;

const fn color(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
const PAPER: u32 = color(255, 255, 253);
const SIDEBAR: u32 = color(241, 246, 238);
const INK: u32 = color(27, 44, 37);
const SOFT: u32 = color(81, 103, 92);
const TEAL: u32 = color(23, 98, 78);
const MINT: u32 = color(221, 237, 226);
const LINE: u32 = color(220, 229, 220);
const AMBER: u32 = color(151, 63, 33);

#[derive(Default)]
struct Widgets {
    search: HWND,
    tree: HWND,
    title: HWND,
    body: HWND,
    preview: HWND,
    all: HWND,
    pinned: HWND,
    trash: HWND,
    new: HWND,
    files: HWND,
    edit: HWND,
    child: HWND,
    block: HWND,
    more: HWND,
    brand: HWND,
    crumb: HWND,
    status: HWND,
    empty: HWND,
    side_status: HWND,
    board: HWND,
    links: HWND,
}

struct App {
    hwnd: HWND,
    widgets: Widgets,
    notebook: Notebook,
    data_dir: PathBuf,
    current: Option<Note>,
    tree_items: HashMap<i64, HTREEITEM>,
    editing: bool,
    dirty: bool,
    editor_ready: bool,
    failed: bool,
    view: View,
    dpi: u32,
    fonts: Vec<HFONT>,
    brushes: [HBRUSH; 2],
    extras: Extras,
}

#[derive(Default)]
struct Extras {
    board: bool,
    cards: Vec<crate::store::Card>,
    dragging: Option<(i64, i32, i32)>,
    scroll: i32,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain([0]).collect()
}

unsafe fn label(hwnd: HWND, text: &str) {
    unsafe {
        SetWindowTextW(hwnd, wide(text).as_ptr());
    }
}

unsafe fn load_editor_text(hwnd: HWND, value: &str) -> bool {
    !value.contains('\0') && unsafe { SetWindowTextW(hwnd, wide(value).as_ptr()) != 0 }
}

unsafe fn text(hwnd: HWND) -> String {
    unsafe { read_text(hwnd).unwrap_or_default() }
}

unsafe fn read_text(hwnd: HWND) -> Result<String> {
    unsafe {
        SetLastError(0);
        let length = GetWindowTextLengthW(hwnd).max(0) as usize;
        if length == 0 && GetLastError() != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut buf = vec![0u16; length + 1];
        let read = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        if length > 0 && read == 0 {
            return Err("無法讀取編輯內容，原始筆記仍保留。".into());
        }
        Ok(String::from_utf16(&buf[..read])?)
    }
}

unsafe fn message(hwnd: HWND, caption: &str, body: &str, flags: u32) -> i32 {
    unsafe { MessageBoxW(hwnd, wide(body).as_ptr(), wide(caption).as_ptr(), flags) }
}

fn workspace_directory(exe: &std::path::Path, portable: bool, local: Option<PathBuf>) -> Result<PathBuf> {
    let adjacent = exe
        .parent()
        .ok_or("找不到程式所在資料夾。")?
        .join("paperroom-data");
    if portable {
        return Ok(adjacent);
    }
    match std::fs::metadata(&adjacent) {
        Ok(metadata) if metadata.is_dir() => return Ok(adjacent),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    let local = local.filter(|path| path.is_absolute())
        .ok_or("LOCALAPPDATA 必須是完整的 Windows 資料夾路徑。請修正後再啟動，或明確選用 --portable。")?;
    Ok(local.join("Paperroom"))
}

fn check_workspace_volume(path: &Path, drive_type: impl FnOnce(&Path) -> u32) -> Result<()> {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err("工作區需要完整的 Windows 路徑。".into());
    };
    let remote = match prefix.kind() {
        Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            let root = PathBuf::from(format!("{}:\\", char::from(letter)));
            drive_type(&root) == 4 // Win32 DRIVE_REMOTE
        }
        _ => return Err("工作區需要本機磁碟路徑。".into()),
    };
    if remote {
        return Err("工作區使用 SQLite WAL，需要本機磁碟。請將工作區移至本機後再開啟。".into());
    }
    Ok(())
}

pub fn run() {
    if let Err(error) = unsafe { start() } {
        unsafe {
            message(null_mut(), "紙間無法啟動", &error.to_string(), MB_ICONERROR);
        }
        std::process::exit(1);
    }
}

// These user32 exports were added in separate Windows 10 feature updates.
// Resolve them from the loaded system DLL so earlier builds can use 96 DPI.
unsafe fn dpi_proc(name: &std::ffi::CStr) -> FARPROC {
    unsafe {
        let module = GetModuleHandleW(wide("user32.dll").as_ptr());
        GetProcAddress(module, name.as_ptr().cast())
    }
}

unsafe fn enable_monitor_dpi() {
    unsafe {
        if let Some(raw) = dpi_proc(c"SetProcessDpiAwarenessContext") {
            let set_context: unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> BOOL =
                std::mem::transmute(raw);
            set_context(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }
}

unsafe fn system_dpi() -> u32 {
    unsafe {
        dpi_proc(c"GetDpiForSystem").map_or(96, |raw| {
            let query: unsafe extern "system" fn() -> u32 = std::mem::transmute(raw);
            query().max(96)
        })
    }
}

unsafe fn window_dpi(hwnd: HWND) -> u32 {
    unsafe {
        dpi_proc(c"GetDpiForWindow").map_or(96, |raw| {
            let query: unsafe extern "system" fn(HWND) -> u32 = std::mem::transmute(raw);
            query(hwnd).max(96)
        })
    }
}

unsafe fn start() -> Result<()> {
    unsafe {
        enable_monitor_dpi();
        let dir = workspace_directory(
            &std::env::current_exe()?,
            std::env::args_os().skip(1).any(|arg| arg == "--portable"),
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        )?;
        check_workspace_volume(&dir, |root| {
            windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide_path(root).as_ptr())
        })?;
        std::fs::create_dir_all(&dir)?;
        let notebook = Notebook::open(&dir.join("notes.sqlite3"), true)?;
        let module = GetModuleHandleW(null());
        if LoadLibraryExW(
            wide("Msftedit.dll").as_ptr(),
            null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
        .is_null()
        {
            return Err("Windows 缺少原生 RichEdit 控制項（Msftedit.dll）。".into());
        }
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TREEVIEW_CLASSES | ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&controls);
        let class = wide("PaperroomWindow");
        let window_class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(dispatch),
            hInstance: module,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&window_class) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let dpi = system_dpi() as i32;
        let scale = |value: i32| value * dpi / 96;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide("紙間").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(1130),
            scale(780),
            null_mut(),
            null_mut(),
            module,
            null(),
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let widgets = Widgets::create(hwnd, module)?;
        let app = Box::new(RefCell::new(App {
            hwnd,
            widgets,
            notebook,
            data_dir: dir,
            current: None,
            tree_items: HashMap::new(),
            editing: false,
            dirty: false,
            editor_ready: false,
            failed: false,
            view: View::All,
            dpi: window_dpi(hwnd),
            fonts: Vec::new(),
            brushes: [CreateSolidBrush(PAPER), CreateSolidBrush(SIDEBAR)],
            extras: Extras::default(),
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*app as *const RefCell<App> as isize);
        {
            let mut state = app.borrow_mut();
            state.scale_fonts();
            state.layout();
            state.rebuild();
            if let Some(first) = state.notebook.rows(View::All, "")?.first() {
                state.select(first.id);
            }
        }
        ShowWindow(hwnd, SW_SHOWNORMAL);
        UpdateWindow(hwnd);
        let mut msg: MSG = std::mem::zeroed();
        loop {
            if REPAINT_PENDING.replace(false) {
                RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ALLCHILDREN);
            }
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result == -1 {
                return Err(std::io::Error::last_os_error().into());
            }
            if result == 0 {
                break;
            }
            if app.borrow_mut().shortcuts(&msg) {
                continue;
            }
            if msg.message == WM_KEYDOWN
                && msg.wParam == VK_TAB as usize
                && msg.hwnd != app.borrow().widgets.body
                && IsDialogMessageW(hwnd, &msg) != 0
            {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}

impl Widgets {
    unsafe fn create(hwnd: HWND, module: HINSTANCE) -> Result<Self> {
        unsafe {
            let control =
                |class: &str, value: &str, id: u16, style: u32, ex: u32| -> Result<HWND> {
                    let child = CreateWindowExW(
                        ex,
                        wide(class).as_ptr(),
                        wide(value).as_ptr(),
                        WS_VISIBLE | WS_CHILD | style,
                        0,
                        0,
                        10,
                        10,
                        hwnd,
                        id as usize as HMENU,
                        module,
                        null(),
                    );
                    if child.is_null() {
                        return Err(std::io::Error::last_os_error().into());
                    }
                    Ok(child)
                };
            let button =
                |value, id| control("BUTTON", value, id, BS_OWNERDRAW as u32 | WS_TABSTOP, 0);
            let plain = |value, id| control("STATIC", value, id, SS_LEFTNOWORDWRAP, 0);
            let widgets = Self {
                search: control(
                    "EDIT",
                    "",
                    SEARCH,
                    WS_TABSTOP | ES_AUTOHSCROLL as u32,
                    WS_EX_CLIENTEDGE,
                )?,
                tree: control(
                    "SysTreeView32",
                    "",
                    TREE,
                    WS_TABSTOP
                        | TVS_HASBUTTONS
                        | TVS_LINESATROOT
                        | TVS_SHOWSELALWAYS
                        | TVS_NOHSCROLL,
                    0,
                )?,
                title: control("EDIT", "", TITLE, WS_TABSTOP | ES_AUTOHSCROLL as u32, 0)?,
                body: control(
                    "EDIT",
                    "",
                    BODY,
                    WS_TABSTOP
                        | WS_VSCROLL
                        | (ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32,
                    0,
                )?,
                preview: control(
                    "RICHEDIT50W",
                    "",
                    PREVIEW,
                    WS_TABSTOP | WS_VSCROLL | (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32,
                    0,
                )?,
                all: button("全部", ALL)?,
                pinned: button("已標星", PINNED)?,
                trash: button("垃圾桶", TRASH)?,
                new: button("＋　新增筆記", NEW)?,
                files: button("檔案", FILES)?,
                edit: button("編輯", EDIT)?,
                child: button("新增子頁", CHILD)?,
                block: button("插入區塊", BLOCK)?,
                more: button("更多", MORE)?,
                brand: plain("紙間", BRAND)?,
                crumb: plain("筆記", BREADCRUMB)?,
                status: plain("", STATUS)?,
                empty: plain("從左側選一頁，或建立一篇新筆記。", EMPTY)?,
                side_status: plain("●　本機工作區", SIDE_STATUS)?,
                board: button("卡片白板", BOARD)?,
                links: button("連結", LINKS)?,
            };
            SendMessageW(
                widgets.search,
                EM_SETCUEBANNER,
                1,
                wide("搜尋標題與內容").as_ptr() as isize,
            );
            SendMessageW(widgets.search, EM_SETLIMITTEXT, 180, 0);
            SendMessageW(widgets.title, EM_SETLIMITTEXT, MAX_TITLE_CHARS * 2, 0);
            SendMessageW(
                widgets.title,
                EM_SETCUEBANNER,
                1,
                wide("未命名筆記").as_ptr() as isize,
            );
            SendMessageW(widgets.title, EM_SETMARGINS, EC_LEFTMARGIN as usize, 0);
            // Native EDIT accepts only plain text, including clipboard and WM_SETTEXT input.
            // CRLF expansion can double the size of an existing LF-only note.
            SendMessageW(widgets.body, EM_SETLIMITTEXT, MAX_NOTE_BYTES * 2, 0);
            SendMessageW(
                widgets.preview,
                EM_EXLIMITTEXT,
                0,
                (MAX_NOTE_BYTES * 2) as isize,
            );
            SendMessageW(widgets.preview, EM_SETBKGNDCOLOR, 0, PAPER as isize);
            SendMessageW(widgets.tree, TVM_SETBKCOLOR, 0, SIDEBAR as isize);
            SendMessageW(widgets.tree, TVM_SETTEXTCOLOR, 0, INK as isize);
            SendMessageW(widgets.tree, TVM_SETLINECOLOR, 0, SOFT as isize);
            Ok(widgets)
        }
    }
}

impl App {
    fn scaled(&self, x: i32) -> i32 {
        (x * self.dpi as i32 + 48) / 96
    }

    unsafe fn scale_fonts(&mut self) {
        unsafe {
            let w = &self.widgets;
            let make = |px, weight| {
                CreateFontW(
                    -self.scaled(px),
                    0,
                    0,
                    0,
                    weight,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    DEFAULT_PITCH as u32,
                    wide("Microsoft JhengHei UI").as_ptr(),
                )
            };
            let fonts = vec![
                make(16, 400),
                make(38, 700),
                make(32, 700),
                make(14, 400),
                make(18, 400),
            ];
            for (controls, index) in [
                (
                    vec![
                        w.search, w.tree, w.new, w.files, w.all, w.pinned, w.trash, w.edit,
                        w.child, w.block, w.more, w.board, w.links,
                    ],
                    0,
                ),
                (vec![w.title], 1),
                (vec![w.brand], 2),
                (vec![w.crumb, w.status, w.side_status], 3),
                (vec![w.body, w.preview, w.empty], 4),
            ] {
                for child in controls {
                    SendMessageW(child, WM_SETFONT, fonts[index] as usize, 1);
                }
            }
            SendMessageW(w.tree, TVM_SETITEMHEIGHT, self.scaled(34) as usize, 0);
            SendMessageW(w.tree, TVM_SETINDENT, self.scaled(22) as usize, 0);
            for font in self.fonts.drain(..) {
                DeleteObject(font);
            }
            self.fonts = fonts;
        }
    }

    unsafe fn layout(&self) {
        unsafe {
            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rect);
            let width = rect.right * 96 / self.dpi as i32;
            let height = rect.bottom * 96 / self.dpi as i32;
            let sidebar = 265;
            let margin = if width >= 1030 { 65 } else { 38 };
            let doc_left = sidebar + margin;
            let doc_width = (width - doc_left - margin).max(1);
            let place = |hwnd, x, y, width: i32, height: i32| {
                MoveWindow(
                    hwnd,
                    self.scaled(x),
                    self.scaled(y),
                    self.scaled(width.max(1)),
                    self.scaled(height.max(1)),
                    1,
                );
            };
            let w = &self.widgets;
            place(w.brand, 22, 24, 140, 49);
            place(w.files, 166, 33, 76, 34);
            place(w.search, 20, 87, 224, 30);
            place(w.new, 20, 130, 224, 38);
            place(w.all, 20, 187, 63, 30);
            place(w.pinned, 90, 187, 72, 30);
            place(w.trash, 169, 187, 75, 30);
            place(w.tree, 18, 231, 231, height - 330);
            place(w.board, 20, height - 91, 224, 35);
            place(w.side_status, 22, height - 39, 220, 25);
            place(w.crumb, doc_left, 36, doc_width, 23);
            place(w.title, doc_left, 65, doc_width, 56);
            place(w.edit, doc_left, 134, 77, 34);
            place(w.child, doc_left + 84, 134, 98, 34);
            place(w.block, doc_left + 188, 134, 98, 34);
            if self.extras.board {
                place(w.links, doc_left, 62, 102, 32);
            } else {
                place(w.links, doc_left + 291, 134, 74, 34);
            }
            place(w.more, doc_left + 370, 134, 70, 34);
            place(w.body, doc_left, 190, doc_width, height - 241);
            place(w.preview, doc_left, 190, doc_width, height - 241);
            place(w.status, doc_left, height - 35, doc_width, 23);
            place(w.empty, doc_left, 185, doc_width, 50);
        }
    }

    unsafe fn rebuild(&mut self) {
        unsafe {
            let tree = self.widgets.tree;
            let query = text(self.widgets.search);
            let rows = match self.notebook.rows(self.view, &query) {
                Ok(rows) => rows,
                Err(error) => {
                    self.show_error(error);
                    return;
                }
            };
            let expanded: HashSet<i64> = self
                .tree_items
                .iter()
                .filter_map(|(&id, &item)| {
                    let mask = TVIS_EXPANDED as usize;
                    (SendMessageW(tree, TVM_GETITEMSTATE, item as usize, mask as isize) as usize
                        & mask
                        != 0)
                        .then_some(id)
                })
                .collect();
            let first_draw = self.tree_items.is_empty();
            self.tree_items.clear();
            SendMessageW(tree, WM_SETREDRAW, 0, 0);
            SendMessageW(tree, TVM_DELETEITEM, 0, TVI_ROOT as isize);
            let nested = self.view == View::All && query.trim().is_empty();
            let mut branches: HashMap<Option<i64>, Vec<&Row>> = HashMap::new();
            for row in &rows {
                branches
                    .entry(if nested { row.parent } else { None })
                    .or_default()
                    .push(row);
            }
            let mut pending = vec![(None, TVI_ROOT)];
            while let Some((parent, handle)) = pending.pop() {
                let Some(branch) = branches.get(&parent) else {
                    continue;
                };
                for row in branch {
                    let mut display = wide(&if row.pinned {
                        format!("★ {}", row.title)
                    } else {
                        row.title.clone()
                    });
                    let item = TVITEMW {
                        mask: TVIF_TEXT | TVIF_PARAM,
                        pszText: display.as_mut_ptr(),
                        lParam: row.id as isize,
                        ..std::mem::zeroed()
                    };
                    let descriptor = TVINSERTSTRUCTW {
                        hParent: handle,
                        hInsertAfter: TVI_LAST,
                        Anonymous: TVINSERTSTRUCTW_0 { item },
                    };
                    let inserted =
                        SendMessageW(tree, TVM_INSERTITEMW, 0, &descriptor as *const _ as isize)
                            as HTREEITEM;
                    self.tree_items.insert(row.id, inserted);
                    pending.push((Some(row.id), inserted));
                }
                if let Some(id) = parent
                    && (expanded.contains(&id) || first_draw)
                {
                    // Expand after the child handles have been created.
                    SendMessageW(tree, TVM_EXPAND, TVE_EXPAND as usize, handle as isize);
                }
            }
            self.select_tree();
            SendMessageW(tree, WM_SETREDRAW, 1, 0);
            InvalidateRect(tree, null(), 1);
            let hint = if !query.trim().is_empty() {
                format!("{} 筆搜尋結果", rows.len())
            } else if self.data_dir.ends_with("paperroom-data") {
                "●　可攜工作區".to_owned()
            } else {
                "●　本機工作區".to_owned()
            };
            label(self.widgets.side_status, &hint);
        }
    }

    unsafe fn select_tree(&self) {
        unsafe {
            let handle = self
                .current
                .as_ref()
                .and_then(|n| self.tree_items.get(&n.row.id).copied())
                .unwrap_or(0);
            SendMessageW(
                self.widgets.tree,
                TVM_SELECTITEM,
                TVGN_CARET as usize,
                handle as isize,
            );
            if handle != 0 {
                SendMessageW(self.widgets.tree, TVM_ENSUREVISIBLE, 0, handle as isize);
            }
        }
    }

    unsafe fn select(&mut self, id: i64) {
        unsafe {
            if self.current.as_ref().is_some_and(|note| note.row.id == id) {
                if self.extras.board {
                    self.extras.board = false;
                    self.present();
                }
                return;
            }
            if !self.flush(true) {
                self.select_tree();
                return;
            }
            match self.notebook.note(id) {
                Ok(note) => {
                    self.current = note;
                    self.extras.board = false;
                    self.dirty = false;
                    self.failed = false;
                    self.present();
                    self.select_tree();
                }
                Err(error) => self.show_error(error),
            }
        }
    }

    unsafe fn present(&mut self) {
        unsafe {
            let w = &self.widgets;
            let board = self.extras.board;
            let note = self.current.as_ref().filter(|_| !board);
            let active = note.is_some_and(|n| n.row.trash_root.is_none());
            let editing = active && self.editing;
            self.editor_ready = false;
            for (item, visible) in [
                (w.title, note.is_some()),
                (w.body, editing),
                (w.preview, note.is_some() && !editing),
                (w.empty, note.is_none() && !board),
                (w.edit, active),
                (w.child, active),
                (w.block, active),
                (w.links, active || board),
                (w.more, note.is_some()),
            ] {
                ShowWindow(item, if visible { SW_SHOW } else { SW_HIDE });
            }
            EnableWindow(w.block, i32::from(editing));
            label(
                w.board,
                if board {
                    "返回筆記"
                } else {
                    "卡片白板"
                },
            );
            label(w.links, if board { "放入白板" } else { "連結" });
            if board {
                label(w.crumb, "卡片白板");
                label(
                    w.status,
                    "拖曳卡片調整位置　雙擊開啟筆記　右鍵移出白板　滾輪捲動",
                );
            } else if let Some(note) = note {
                let title_loaded = load_editor_text(w.title, &note.row.title);
                SendMessageW(w.title, EM_SETREADONLY, usize::from(!editing), 0);
                if editing {
                    let body = note.body.replace('\n', "\r\n");
                    self.editor_ready = load_editor_text(w.body, &body) && title_loaded;
                    EnableWindow(w.title, i32::from(self.editor_ready));
                    EnableWindow(w.body, i32::from(self.editor_ready));
                    SendMessageW(w.body, EM_EMPTYUNDOBUFFER, 0, 0);
                } else {
                    SendMessageW(w.preview, EM_SETREADONLY, 0, 0);
                    set_rtf(w.preview, &crate::markdown::preview(&note.body));
                    SendMessageW(w.preview, EM_SETREADONLY, 1, 0);
                }
                label(w.crumb, &self.breadcrumb(note));
                label(w.edit, if editing { "閱讀" } else { "編輯" });
                label(w.more, if active { "更多" } else { "還原" });
                label(
                    w.status,
                    if editing && !self.editor_ready {
                        "編輯器載入失敗，原始筆記仍保留。請重新開啟本頁。"
                    } else if active {
                        "已儲存至此電腦"
                    } else {
                        "此頁在垃圾桶"
                    },
                );
            } else {
                label(
                    w.crumb,
                    match self.view {
                        View::All => "筆記",
                        View::Pinned => "已標星",
                        View::Trash => "垃圾桶",
                    },
                );
                label(
                    w.empty,
                    if self.view == View::Trash {
                        "從垃圾桶選一頁即可還原。"
                    } else {
                        "從左側選一頁，或建立一篇新筆記。"
                    },
                );
                label(w.status, "");
            }
            self.layout();
            InvalidateRect(self.hwnd, null(), 1);
        }
    }

    fn breadcrumb(&self, note: &Note) -> String {
        if note.row.trash_root.is_some() {
            return "垃圾桶".to_owned();
        }
        let mut titles = Vec::new();
        let mut parent = note.row.parent;
        for _ in 0..64 {
            let Some(id) = parent else {
                break;
            };
            let Ok(Some(row)) = self.notebook.meta(id) else {
                break;
            };
            titles.push(row.title);
            parent = row.parent;
        }
        titles.push("筆記".to_owned());
        titles.reverse();
        titles.join("　/　")
    }

    unsafe fn mark_dirty(&mut self) {
        unsafe {
            if !self.editing
                || !self.editor_ready
                || self.extras.board
                || self
                    .current
                    .as_ref()
                    .is_none_or(|n| n.row.trash_root.is_some())
            {
                return;
            }
            self.dirty = true;
            KillTimer(self.hwnd, SAVE_TIMER);
            label(self.widgets.status, "有尚未儲存的修改…");
            if SetTimer(self.hwnd, SAVE_TIMER, 550, None) == 0 {
                self.flush(false);
            }
        }
    }

    unsafe fn flush(&mut self, show_dialog: bool) -> bool {
        unsafe {
            if !self.dirty {
                return true;
            }
            KillTimer(self.hwnd, SAVE_TIMER);
            let Some(note) = self.current.as_mut() else {
                return true;
            };
            let read_and_save = || -> Result<_> {
                let title = read_text(self.widgets.title)?;
                let body = read_text(self.widgets.body)?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n");
                let version = self
                    .notebook
                    .save(note.row.id, note.version, &title, &body)?;
                Ok((version, title, body))
            };
            match read_and_save() {
                Ok((version, title, body)) => {
                    let title = crate::store::clean_title(&title);
                    let changed = note.row.title != title;
                    note.version = version;
                    note.row.title = title;
                    note.body = body;
                    self.dirty = false;
                    self.failed = false;
                    label(self.widgets.status, "已儲存至此電腦");
                    if changed {
                        self.rebuild();
                    }
                    true
                }
                Err(error) => {
                    self.failed = true;
                    label(self.widgets.status, &format!("儲存失敗　{error}"));
                    if show_dialog {
                        self.show_error(error);
                    }
                    false
                }
            }
        }
    }

    unsafe fn mutate(&mut self, f: impl FnOnce(&mut Self) -> Result<()>) {
        unsafe {
            if !self.flush(true) {
                return;
            }
            if let Err(error) = f(self) {
                self.show_error(error);
            }
        }
    }

    unsafe fn new_note(&mut self, child: bool) {
        unsafe {
            self.mutate(|app| {
                let parent = if child {
                    app.current.as_ref().map(|n| n.row.id)
                } else {
                    None
                };
                let id = app.notebook.add(parent, "未命名筆記", "")?;
                app.view = View::All;
                label(app.widgets.search, "");
                app.editing = true;
                app.select(id);
                app.rebuild();
                SetFocus(app.widgets.title);
                SendMessageW(app.widgets.title, EM_SETSEL, 0, -1);
                Ok(())
            });
        }
    }

    unsafe fn change_view(&mut self, view: View) {
        unsafe {
            self.mutate(|app| {
                app.view = view;
                app.current = None;
                app.extras.board = false;
                label(app.widgets.search, "");
                app.rebuild();
                if let Some(id) = app.notebook.rows(view, "")?.first().map(|r| r.id) {
                    app.select(id);
                } else {
                    app.present();
                }
                for button in [app.widgets.all, app.widgets.pinned, app.widgets.trash] {
                    InvalidateRect(button, null(), 1);
                }
                Ok(())
            });
        }
    }

    unsafe fn toggle_edit(&mut self) {
        unsafe {
            if self
                .current
                .as_ref()
                .is_none_or(|n| n.row.trash_root.is_some())
                || self.extras.board
            {
                return;
            }
            if self.flush(true) {
                self.editing = !self.editing;
                self.present();
                if self.editing {
                    SetFocus(self.widgets.body);
                }
            }
        }
    }

    unsafe fn show_error(&self, error: Box<dyn std::error::Error>) {
        unsafe {
            message(self.hwnd, "紙間", &error.to_string(), MB_ICONERROR);
        }
    }

    unsafe fn command(&mut self, id: u16, code: u16) {
        unsafe {
            if matches!(id, TITLE | BODY) && code == EN_CHANGE as u16 {
                self.mark_dirty();
                return;
            }
            if id == SEARCH && code == EN_CHANGE as u16 {
                self.rebuild();
                return;
            }
            if code != BN_CLICKED as u16 {
                return;
            }
            match id {
                ALL => self.change_view(View::All),
                PINNED => self.change_view(View::Pinned),
                TRASH => self.change_view(View::Trash),
                NEW => self.new_note(false),
                CHILD => self.new_note(true),
                EDIT => self.toggle_edit(),
                FILES => self.file_menu(),
                BLOCK => {
                    self.block_menu(false);
                }
                MORE => self.more_menu(),
                BOARD => self.board_toggle(),
                LINKS => self.links_menu(),
                _ => {}
            }
        }
    }

    unsafe fn file_menu(&mut self) {
        unsafe {
            let items = [
                (1, "匯入 Markdown…"),
                (2, "匯入 Notion Markdown ZIP…"),
                (3, "匯出目前筆記為 Markdown…"),
                (4, "備份工作區…"),
                (5, "開啟資料所在資料夾"),
            ];
            let action = popup(self.hwnd, self.widgets.files, None, &items);
            self.mutate(|app| {
                let action = match action {
                    1 => ("Markdown 筆記\0*.md;*.markdown\0\0", "md", None),
                    2 => ("Notion Markdown ZIP\0*.zip\0\0", "zip", None),
                    3 => ("Markdown 筆記\0*.md\0\0", "md", Some("note.md")),
                    4 => (
                        "紙間工作區備份\0*.sqlite3\0\0",
                        "sqlite3",
                        Some("paperroom-backup.sqlite3"),
                    ),
                    5 => {
                        let opened = ShellExecuteW(
                            app.hwnd,
                            wide("open").as_ptr(),
                            wide_path(&app.data_dir).as_ptr(),
                            null(),
                            null(),
                            SW_SHOWNORMAL,
                        );
                        if (opened as usize) <= 32 {
                            return Err("無法開啟工作區資料夾。".into());
                        }
                        return Ok(());
                    }
                    _ => return Ok(()),
                };
                if let Some(path) = file_dialog(app.hwnd, action.0, action.1, action.2)? {
                    match action.1 {
                        "md" if action.2.is_none() => {
                            let page = archive::markdown(&path)?;
                            let id = app.notebook.add(None, &page.title, &page.body)?;
                            app.open_import(id, "已匯入 1 篇 Markdown");
                        }
                        "zip" => {
                            let imported = archive::archive(&path)?;
                            let report = format!(
                                "已匯入 {} 篇 Markdown　略過 {} 個其他項目",
                                imported.pages.len(),
                                imported.skipped
                            );
                            let name = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Notion ZIP");
                            let id = app
                                .notebook
                                .import(&format!("匯入：{name}"), imported.pages)?;
                            app.open_import(id, &report);
                        }
                        "md" => {
                            let note = app.current.as_ref().ok_or("先選擇要匯出的筆記。")?;
                            archive::export_note(&path, &note.row.title, &note.body)?;
                            label(app.widgets.status, "Markdown 已匯出");
                        }
                        _ => {
                            app.notebook.backup(&path)?;
                            label(app.widgets.status, "工作區備份已完成");
                        }
                    }
                }
                Ok(())
            });
        }
    }

    unsafe fn open_import(&mut self, id: i64, info: &str) {
        unsafe {
            self.view = View::All;
            self.editing = false;
            label(self.widgets.search, "");
            self.rebuild();
            self.select(id);
            label(self.widgets.status, info);
        }
    }

    unsafe fn more_menu(&mut self) {
        unsafe {
            self.mutate(|app| {
                let Some(note) = &app.current else {
                    return Ok(());
                };
                let id = note.row.id;
                if note.row.trash_root.is_some() {
                    let restored = app.notebook.restore(id)?;
                    app.change_view(View::All);
                    app.select(restored);
                    return Ok(());
                }
                let star = if note.row.pinned {
                    "取消標星"
                } else {
                    "標星"
                };
                let items = [(1, star), (2, "移到…"), (3, "移到垃圾桶")];
                match popup(app.hwnd, app.widgets.more, None, &items) {
                    1 => {
                        app.notebook.pin(id)?;
                        app.current = app.notebook.note(id)?;
                        app.rebuild();
                    }
                    2 => app.move_menu(id)?,
                    3 => {
                        if message(
                            app.hwnd,
                            "移到垃圾桶",
                            "將這頁及其子頁移到垃圾桶？稍後可以還原。",
                            MB_OKCANCEL | MB_ICONQUESTION,
                        ) == IDOK
                        {
                            app.notebook.trash(id)?;
                            app.change_view(app.view);
                        }
                    }
                    _ => {}
                }
                Ok(())
            });
        }
    }

    unsafe fn move_menu(&mut self, id: i64) -> Result<()> {
        unsafe {
            let rows = self.notebook.rows(View::All, "")?;
            let mut excluded = HashSet::from([id]);
            while rows.iter().any(|row| {
                !excluded.contains(&row.id) && row.parent.is_some_and(|p| excluded.contains(&p))
            }) {
                for row in &rows {
                    if row.parent.is_some_and(|p| excluded.contains(&p)) {
                        excluded.insert(row.id);
                    }
                }
            }
            let choices = rows
                .iter()
                .filter(|r| !excluded.contains(&r.id))
                .collect::<Vec<_>>();
            let mut items = vec![(1, "工作區最上層")];
            for (index, row) in choices.iter().enumerate() {
                items.push((index + 2, row.title.as_str()));
            }
            let choice = popup(self.hwnd, self.widgets.more, None, &items);
            if choice > 0 {
                self.notebook.relocate(
                    id,
                    if choice == 1 {
                        None
                    } else {
                        Some(choices[choice - 2].id)
                    },
                )?;
                self.current = self.notebook.note(id)?;
                self.present();
                self.rebuild();
            }
            Ok(())
        }
    }

    unsafe fn block_menu(&mut self, slash: bool) -> bool {
        unsafe {
            if !self.editing
                || self.extras.board
                || self
                    .current
                    .as_ref()
                    .is_none_or(|n| n.row.trash_root.is_some())
            {
                return false;
            }
            let items = [
                (1, "大標題"),
                (2, "小標題"),
                (3, "項目清單"),
                (4, "編號清單"),
                (5, "待辦事項"),
                (6, "引用"),
                (7, "程式碼"),
                (8, "分隔線"),
                (9, "切換本行待辦狀態"),
            ];
            let point = if slash {
                let mut pt = POINT { x: 0, y: 0 };
                GetCaretPos(&mut pt);
                pt.y += self.scaled(24);
                ClientToScreen(self.widgets.body, &mut pt);
                Some(pt)
            } else {
                None
            };
            let item = popup(self.hwnd, self.widgets.block, point, &items);
            let templates = [
                "# ",
                "## ",
                "- ",
                "1. ",
                "- [ ] ",
                "> ",
                "```\r\n\r\n```",
                "---\r\n",
            ];
            if let Some(&snippet) = item.checked_sub(1).and_then(|i| templates.get(i)) {
                self.replace_selection(snippet);
                true
            } else if item == 9 {
                self.toggle_task();
                true
            } else {
                false
            }
        }
    }

    unsafe fn replace_selection(&mut self, value: &str) {
        unsafe {
            SetFocus(self.widgets.body);
            SendMessageW(
                self.widgets.body,
                EM_REPLACESEL,
                1,
                wide(value).as_ptr() as isize,
            );
            self.mark_dirty();
        }
    }

    unsafe fn caret_line(&self) -> (i32, i32) {
        unsafe {
            let line = SendMessageW(self.widgets.body, EM_LINEFROMCHAR, usize::MAX, 0);
            let start = SendMessageW(self.widgets.body, EM_LINEINDEX, line as usize, 0) as i32;
            let length = SendMessageW(self.widgets.body, EM_LINELENGTH, start as usize, 0) as i32;
            (start, length)
        }
    }

    unsafe fn toggle_task(&mut self) {
        unsafe {
            let (start, length) = self.caret_line();
            if start < 0 || !(5..=MAX_NOTE_BYTES as i32).contains(&length) {
                return;
            }
            let mut buffer = vec![length as u16; length.min(65_534) as usize + 1];
            let line = SendMessageW(self.widgets.body, EM_LINEFROMCHAR, usize::MAX, 0);
            buffer[0] = (buffer.len() - 1) as u16;
            let count = SendMessageW(
                self.widgets.body,
                EM_GETLINE,
                line as usize,
                buffer.as_mut_ptr() as isize,
            ) as usize;
            let Some(line) = buffer.get(..count) else {
                return;
            };
            let line = String::from_utf16_lossy(line);
            let n = line.len() - line.trim_start_matches([' ', '\t']).len();
            let Some((_, mark)) = ["- [ ]", "- [x]", "- [X]", "* [ ]", "* [x]", "* [X]"]
                .iter()
                .enumerate()
                .find(|(_, mark)| line[n..].starts_with(**mark))
            else {
                return;
            };
            let at = start + n as i32 + 3;
            SendMessageW(self.widgets.body, EM_SETSEL, at as usize, (at + 1) as isize);
            self.replace_selection(if mark.as_bytes()[3] == b' ' { "x" } else { " " });
        }
    }

    unsafe fn shortcuts(&mut self, msg: &MSG) -> bool {
        unsafe {
            if msg.message == WM_CHAR
                && msg.wParam == b'/' as usize
                && msg.hwnd == self.widgets.body
                && self.editing
                && self.caret_line().1 == 0
            {
                return self.block_menu(true);
            }
            if msg.message != WM_KEYDOWN || GetKeyState(VK_CONTROL as i32) >= 0 {
                return false;
            }
            match msg.wParam as u8 {
                b'N' => self.new_note(false),
                b'F' => {
                    SetFocus(self.widgets.search);
                    SendMessageW(self.widgets.search, EM_SETSEL, 0, -1);
                }
                b'S' => {
                    self.flush(true);
                }
                b'E' => self.toggle_edit(),
                _ => return false,
            }
            true
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            for font in &self.fonts {
                DeleteObject(*font);
            }
            for brush in &self.brushes {
                DeleteObject(*brush);
            }
        }
    }
}

#[repr(C, packed(4))]
struct EditStream {
    cookie: usize,
    error: u32,
    callback: Option<unsafe extern "system" fn(usize, *mut u8, i32, *mut i32) -> u32>,
}

unsafe extern "system" fn stream(
    cookie: usize,
    output: *mut u8,
    length: i32,
    wrote: *mut i32,
) -> u32 {
    unsafe {
        let input = &mut *(cookie as *mut &[u8]);
        let count = input.len().min(length.max(0) as usize);
        std::ptr::copy_nonoverlapping(input.as_ptr(), output, count);
        *wrote = count as i32;
        *input = &input[count..];
        0
    }
}

unsafe fn set_rtf(hwnd: HWND, value: &str) {
    unsafe {
        let mut bytes = value.as_bytes();
        let mut data = EditStream {
            cookie: &mut bytes as *mut _ as usize,
            error: 0,
            callback: Some(stream),
        };
        SendMessageW(hwnd, EM_STREAMIN, SF_RTF, &mut data as *mut _ as isize);
    }
}

unsafe fn popup(hwnd: HWND, anchor: HWND, point: Option<POINT>, items: &[(usize, &str)]) -> usize {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        for (id, title) in items {
            AppendMenuW(
                menu,
                MF_STRING,
                *id,
                wide(&title.replace('&', "&&")).as_ptr(),
            );
        }
        let pt = point.unwrap_or_else(|| {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(anchor, &mut r);
            POINT {
                x: r.left,
                y: r.bottom + 3,
            }
        });
        let selection = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            null(),
        );
        DestroyMenu(menu);
        selection as usize
    }
}

unsafe fn file_dialog(
    hwnd: HWND,
    filter: &str,
    extension: &str,
    name: Option<&str>,
) -> Result<Option<PathBuf>> {
    unsafe {
        use std::os::windows::ffi::OsStringExt;
        let mut buffer = vec![0u16; 32_768];
        if let Some(name) = name {
            let initial = wide(name);
            buffer[..initial.len()].copy_from_slice(&initial);
        }
        let filter = wide(filter);
        let extension = wide(extension);
        let mut dialog = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: hwnd,
            lpstrFilter: filter.as_ptr(),
            lpstrFile: buffer.as_mut_ptr(),
            nMaxFile: buffer.len() as u32,
            lpstrDefExt: extension.as_ptr(),
            Flags: OFN_NOCHANGEDIR
                | OFN_PATHMUSTEXIST
                | OFN_EXPLORER
                | OFN_HIDEREADONLY
                | OFN_DONTADDTORECENT
                | if name.is_none() { OFN_FILEMUSTEXIST } else { 0 },
            ..std::mem::zeroed()
        };
        let selected = if name.is_some() {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        };
        if selected == 0 {
            let error = CommDlgExtendedError();
            if error != 0 {
                return Err(format!("無法開啟檔案對話框（{error}）。").into());
            }
            return Ok(None);
        }
        let end = buffer.iter().position(|i| *i == 0).unwrap_or(buffer.len());
        Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
            &buffer[..end],
        ))))
    }
}

impl App {
    unsafe fn links_menu(&mut self) {
        unsafe {
            if self.extras.board {
                self.mutate(|app| {
                    let choices = app.notebook.rows(View::All, "")?;
                    if choices.is_empty() {
                        return Ok(());
                    }
                    let items = choices
                        .iter()
                        .enumerate()
                        .map(|(i, row)| (i + 1, row.title.as_str()))
                        .collect::<Vec<_>>();
                    let selection = popup(app.hwnd, app.widgets.links, None, &items);
                    if selection > 0 {
                        app.notebook.add_card(choices[selection - 1].id)?;
                        app.refresh_board()?;
                    }
                    Ok(())
                });
                return;
            }
            self.mutate(|app| {
                let Some(note) = &app.current else {
                    return Ok(());
                };
                let (outgoing, incoming) = app.notebook.related(note)?;
                let mut labels = vec![(1, "插入筆記連結…".to_owned())];
                let related = outgoing
                    .iter()
                    .map(|row| ("前往", row))
                    .chain(incoming.iter().map(|row| ("反向連結", row)))
                    .collect::<Vec<_>>();
                for (index, (name, row)) in related.iter().enumerate() {
                    labels.push((index + 2, format!("{name}　{}", row.title)));
                }
                let items = labels
                    .iter()
                    .map(|(id, label)| (*id, label.as_str()))
                    .collect::<Vec<_>>();
                match popup(app.hwnd, app.widgets.links, None, &items) {
                    0 => {}
                    1 => {
                        let choices = app.notebook.rows(View::All, "")?;
                        let items = choices
                            .iter()
                            .enumerate()
                            .map(|(i, row)| (i + 1, row.title.as_str()))
                            .collect::<Vec<_>>();
                        let selection = popup(app.hwnd, app.widgets.links, None, &items);
                        if selection > 0 {
                            let target = &choices[selection - 1];
                            if !app.editing {
                                app.toggle_edit();
                            }
                            let name = target.title.replace(['[', ']', '|'], " ");
                            app.replace_selection(&format!("[[#{}|{name}]]", target.id));
                        }
                    }
                    index => {
                        let id = related[index - 2].1.id;
                        if !app.tree_items.contains_key(&id) {
                            app.view = View::All;
                            label(app.widgets.search, "");
                            app.rebuild();
                        }
                        app.select(id);
                    }
                }
                Ok(())
            });
        }
    }

    unsafe fn refresh_board(&mut self) -> Result<()> {
        unsafe {
            self.extras.cards = self.notebook.cards()?;
            let end = self.extras.cards.iter().map(|c| c.y).max().unwrap_or(0);
            self.extras.scroll = self.extras.scroll.clamp(0, end);
            InvalidateRect(self.hwnd, null(), 1);
            Ok(())
        }
    }

    unsafe fn board_toggle(&mut self) {
        unsafe {
            self.mutate(|app| {
                app.extras.board = !app.extras.board;
                if app.extras.board {
                    app.refresh_board()?;
                }
                app.present();
                Ok(())
            });
        }
    }

    unsafe fn board_area(&self) -> RECT {
        unsafe {
            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rect);
            rect.left = self.scaled(if rect.right >= self.scaled(1030) {
                330
            } else {
                303
            });
            rect.top = self.scaled(100);
            rect.right -= self.scaled(27);
            rect.bottom -= self.scaled(49);
            rect
        }
    }

    unsafe fn card_at(&self, x: i32, y: i32) -> Option<i64> {
        unsafe {
            let area = self.board_area();
            if x < area.left || y < area.top || x >= area.right || y >= area.bottom {
                return None;
            }
            self.extras
                .cards
                .iter()
                .rev()
                .find(|card| {
                    let left = area.left + self.scaled(card.x);
                    let top = area.top + self.scaled(card.y - self.extras.scroll);
                    (left..left + self.scaled(232)).contains(&x)
                        && (top..top + self.scaled(114)).contains(&y)
                })
                .map(|card| card.id)
        }
    }

    unsafe fn board_input(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) {
        unsafe {
            if !self.extras.board {
                return;
            }
            let x = lp as u16 as i16 as i32;
            let y = (lp >> 16) as u16 as i16 as i32;
            match msg {
                WM_LBUTTONDOWN => {
                    if let Some(id) = self.card_at(x, y)
                        && let Some(card) = self.extras.cards.iter().find(|c| c.id == id)
                    {
                        let area = self.board_area();
                        self.extras.dragging = Some((
                            id,
                            x - area.left - self.scaled(card.x),
                            y - area.top - self.scaled(card.y - self.extras.scroll),
                        ));
                        SetCapture(self.hwnd);
                    }
                }
                WM_MOUSEMOVE => {
                    if let Some((id, dx, dy)) = self.extras.dragging {
                        let area = self.board_area();
                        let dpi = self.dpi as i32;
                        let max_x = (area.right - area.left) * 96 / dpi - 232;
                        let new_x = ((x - area.left - dx) * 96 / dpi).clamp(0, max_x.max(0));
                        let new_y =
                            ((y - area.top - dy) * 96 / dpi + self.extras.scroll).clamp(0, 50000);
                        if let Some(card) = self.extras.cards.iter_mut().find(|c| c.id == id) {
                            card.x = new_x;
                            card.y = new_y;
                            InvalidateRect(self.hwnd, &area, 1);
                        }
                    }
                }
                WM_LBUTTONUP => {
                    if let Some((id, _, _)) = self.extras.dragging.take() {
                        ReleaseCapture();
                        if let Some(card) = self.extras.cards.iter().find(|c| c.id == id)
                            && let Err(error) = self.notebook.move_card(id, card.x, card.y)
                        {
                            self.show_error(error);
                        }
                    }
                }
                WM_CAPTURECHANGED => {
                    if self.extras.dragging.take().is_some() {
                        let _ = self.refresh_board();
                    }
                }
                WM_LBUTTONDBLCLK => {
                    if let Some(id) = self.card_at(x, y) {
                        self.select(id);
                    }
                }
                WM_RBUTTONUP => {
                    if let Some(id) = self.card_at(x, y) {
                        let mut pt = POINT { x, y };
                        ClientToScreen(self.hwnd, &mut pt);
                        if popup(self.hwnd, null_mut(), Some(pt), &[(1, "移出白板")]) == 1 {
                            if let Err(error) = self
                                .notebook
                                .remove_card(id)
                                .and_then(|_| self.refresh_board())
                            {
                                self.show_error(error);
                            }
                        }
                    }
                }
                WM_MOUSEWHEEL => {
                    let mut cursor = POINT { x, y };
                    ScreenToClient(self.hwnd, &mut cursor);
                    let area = self.board_area();
                    if cursor.x >= area.left && cursor.x < area.right {
                        let delta = (wp >> 16) as u16 as i16 as i32;
                        let max = self.extras.cards.iter().map(|c| c.y).max().unwrap_or(0);
                        self.extras.scroll = (self.extras.scroll - delta).clamp(0, max);
                        InvalidateRect(self.hwnd, &area, 1);
                    }
                }
                _ => {}
            }
        }
    }

    unsafe fn paint(&self) {
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(self.hwnd, &mut ps);
            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(self.hwnd, &mut rect);
            FillRect(dc, &rect, self.brushes[0]);
            let divider = self.scaled(265);
            let mut side = RECT {
                right: divider,
                ..rect
            };
            FillRect(dc, &side, self.brushes[1]);
            side.left = divider;
            side.right = divider + 1;
            fill(dc, &side, LINE);
            if self.extras.board {
                self.paint_board(dc);
            } else if self.current.is_some() {
                let note_margin = self.scaled(if rect.right >= self.scaled(1030) {
                    65
                } else {
                    38
                });
                fill(
                    dc,
                    &RECT {
                        left: divider + note_margin,
                        top: self.scaled(181),
                        right: rect.right - note_margin,
                        bottom: self.scaled(181) + 1,
                    },
                    LINE,
                );
            }
            EndPaint(self.hwnd, &ps);
        }
    }

    unsafe fn paint_board(&self, dc: HDC) {
        unsafe {
            let area = self.board_area();
            let saved = SaveDC(dc);
            IntersectClipRect(dc, area.left, area.top, area.right, area.bottom);
            fill(dc, &area, color(246, 248, 243));
            let step = self.scaled(22).max(1) as usize;
            for y in (area.top..area.bottom).step_by(step) {
                for x in (area.left..area.right).step_by(step) {
                    SetPixelV(dc, x, y, LINE);
                }
            }
            if self.extras.cards.is_empty() {
                let mut guide = RECT {
                    left: area.left + self.scaled(24),
                    top: area.top + self.scaled(24),
                    ..area
                };
                draw_text(
                    dc,
                    self.fonts[4],
                    SOFT,
                    "選擇「放入白板」，把筆記當成卡片排列。",
                    &mut guide,
                    DT_SINGLELINE | DT_END_ELLIPSIS,
                );
            }
            for card in &self.extras.cards {
                let left = area.left + self.scaled(card.x);
                let top = area.top + self.scaled(card.y - self.extras.scroll);
                let rect = RECT {
                    left,
                    top,
                    right: left + self.scaled(232),
                    bottom: top + self.scaled(114),
                };
                if rect.bottom <= area.top || rect.top >= area.bottom {
                    continue;
                }
                fill(dc, &rect, PAPER);
                let brush = CreateSolidBrush(LINE);
                FrameRect(dc, &rect, brush);
                DeleteObject(brush);
                let mut title = RECT {
                    left: left + self.scaled(13),
                    top: top + self.scaled(10),
                    right: rect.right - self.scaled(12),
                    bottom: top + self.scaled(37),
                };
                draw_text(
                    dc,
                    self.fonts[0],
                    TEAL,
                    &card.title,
                    &mut title,
                    DT_SINGLELINE | DT_END_ELLIPSIS,
                );
                let mut excerpt = RECT {
                    top: top + self.scaled(40),
                    bottom: rect.bottom - self.scaled(10),
                    ..title
                };
                draw_text(
                    dc,
                    self.fonts[3],
                    SOFT,
                    &card.excerpt,
                    &mut excerpt,
                    DT_WORDBREAK | DT_END_ELLIPSIS,
                );
            }
            RestoreDC(dc, saved);
        }
    }

    unsafe fn paint_button(&self, draw: &DRAWITEMSTRUCT) {
        unsafe {
            let picked = matches!(
                (draw.CtlID as u16, self.view),
                (ALL, View::All) | (PINNED, View::Pinned) | (TRASH, View::Trash)
            );
            let accent = draw.CtlID == NEW as u32;
            let disabled = draw.itemState & ODS_DISABLED != 0;
            let pressed = draw.itemState & ODS_SELECTED != 0;
            let background = if accent && !disabled {
                if pressed { color(12, 72, 55) } else { TEAL }
            } else if picked || pressed {
                MINT
            } else {
                PAPER
            };
            let foreground = if accent {
                PAPER
            } else if disabled {
                color(112, 125, 115)
            } else {
                INK
            };
            let r = draw.rcItem;
            fill(
                draw.hDC,
                &r,
                if matches!(
                    draw.CtlID as u16,
                    ALL | PINNED | TRASH | FILES | NEW | BOARD
                ) {
                    SIDEBAR
                } else {
                    PAPER
                },
            );
            let set_brush = CreateSolidBrush(background);
            let pen = CreatePen(PS_SOLID, 1, if accent { background } else { LINE });
            let prior_brush = SelectObject(draw.hDC, set_brush);
            let prior_pen = SelectObject(draw.hDC, pen);
            RoundRect(
                draw.hDC,
                r.left,
                r.top,
                r.right,
                r.bottom,
                self.scaled(9),
                self.scaled(9),
            );
            SelectObject(draw.hDC, prior_brush);
            SelectObject(draw.hDC, prior_pen);
            DeleteObject(set_brush);
            DeleteObject(pen);
            let mut target = r;
            draw_text(
                draw.hDC,
                self.fonts[0],
                foreground,
                &text(draw.hwndItem),
                &mut target,
                DT_SINGLELINE | DT_VCENTER | DT_CENTER,
            );
            if draw.itemState & ODS_FOCUS != 0 {
                InflateRect(&mut target, -self.scaled(3), -self.scaled(3));
                DrawFocusRect(draw.hDC, &target);
            }
        }
    }
}

unsafe fn fill(dc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, rect, brush);
        DeleteObject(brush);
    }
}

unsafe fn draw_text(
    dc: HDC,
    font: HFONT,
    foreground: u32,
    value: &str,
    rect: &mut RECT,
    format: u32,
) {
    unsafe {
        let prior = SelectObject(dc, font);
        let mut value = wide(value);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, foreground);
        DrawTextW(
            dc,
            value.as_mut_ptr(),
            (value.len() - 1) as i32,
            rect,
            format | DT_NOPREFIX,
        );
        SelectObject(dc, prior);
    }
}

unsafe extern "system" fn dispatch(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
        let Some(state) = state.as_ref() else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_ERASEBKGND => return 1,
            WM_PAINT => {
                if let Ok(app) = state.try_borrow() {
                    app.paint();
                } else {
                    let mut ps: PAINTSTRUCT = std::mem::zeroed();
                    BeginPaint(hwnd, &mut ps);
                    EndPaint(hwnd, &ps);
                    REPAINT_PENDING.set(true);
                }
                return 0;
            }
            WM_SIZE => {
                if let Ok(app) = state.try_borrow() {
                    app.layout();
                    InvalidateRect(hwnd, null(), 1);
                }
                return 0;
            }
            WM_GETMINMAXINFO => {
                if let Ok(app) = state.try_borrow() {
                    let info = &mut *(lp as *mut MINMAXINFO);
                    info.ptMinTrackSize.x = app.scaled(850);
                    info.ptMinTrackSize.y = app.scaled(570);
                }
                return 0;
            }
            WM_DPICHANGED => {
                if let Ok(mut app) = state.try_borrow_mut() {
                    app.dpi = (wp as u16 as u32).max(96);
                    app.scale_fonts();
                    let target = &*(lp as *const RECT);
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        target.left,
                        target.top,
                        target.right - target.left,
                        target.bottom - target.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    app.layout();
                }
                return 0;
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
                if let Ok(app) = state.try_borrow() {
                    let id = GetDlgCtrlID(lp as HWND) as u16;
                    let sidebar = matches!(id, BRAND | SIDE_STATUS);
                    let dc = wp as HDC;
                    let tone = match id {
                        STATUS if app.failed => AMBER,
                        BRAND | SIDE_STATUS => TEAL,
                        BREADCRUMB | STATUS | EMPTY => SOFT,
                        _ => INK,
                    };
                    SetTextColor(dc, tone);
                    SetBkColor(dc, if sidebar { SIDEBAR } else { PAPER });
                    return app.brushes[usize::from(sidebar)] as isize;
                }
            }
            WM_DRAWITEM => {
                if let Ok(app) = state.try_borrow() {
                    app.paint_button(&*(lp as *const DRAWITEMSTRUCT));
                    return 1;
                }
            }
            WM_COMMAND => {
                if let Ok(mut app) = state.try_borrow_mut() {
                    app.command(wp as u16, (wp >> 16) as u16);
                }
                return 0;
            }
            WM_NOTIFY => {
                let notice = &*(lp as *const NMHDR);
                if notice.idFrom == TREE as usize
                    && notice.code == TVN_SELCHANGEDW
                    && state.try_borrow().is_ok()
                {
                    let id = (*(lp as *const NMTREEVIEWW)).itemNew.lParam;
                    if id > 0 {
                        // Saving a renamed note can rebuild the tree. Wait for its selection
                        // callback to unwind before changing or deleting native item handles.
                        PostMessageW(hwnd, WM_OPEN_TREE_NOTE, id as usize, 0);
                    }
                }
                return 0;
            }
            WM_OPEN_TREE_NOTE => {
                if let Ok(mut app) = state.try_borrow_mut() {
                    app.select(wp as i64);
                }
                return 0;
            }
            WM_TIMER if wp == SAVE_TIMER => {
                if let Ok(mut app) = state.try_borrow_mut() {
                    app.flush(false);
                }
                return 0;
            }
            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE | WM_LBUTTONDBLCLK | WM_RBUTTONUP
            | WM_MOUSEWHEEL | WM_CAPTURECHANGED => {
                if let Ok(mut app) = state.try_borrow_mut() {
                    app.board_input(msg, wp, lp);
                }
            }
            WM_QUERYENDSESSION => {
                // Windows may shut down before the pending autosave timer fires.
                return isize::from(state.try_borrow_mut().is_ok_and(|mut app| app.flush(false)));
            }
            WM_CLOSE => {
                if let Ok(mut app) = state.try_borrow_mut() {
                    if app.flush(true)
                        || message(
                            hwnd,
                            "關閉紙間",
                            "有修改尚未成功儲存。確定要捨棄修改並關閉？",
                            MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING,
                        ) == IDYES
                    {
                        DestroyWindow(hwnd);
                    }
                }
                return 0;
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                PostQuitMessage(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_workspace_rejects_unc_and_mapped_drives() {
        for path in [r"\\server\share\notes", r"\\?\UNC\server\share\notes"] {
            assert!(check_workspace_volume(Path::new(path), |_| unreachable!()).is_err());
        }
        assert!(check_workspace_volume(Path::new(r"Z:\notes"), |root| {
            assert_eq!(root, Path::new(r"Z:\"));
            4
        }).is_err());
        assert!(check_workspace_volume(Path::new(r"\\?\C:\notes"), |_| 3).is_ok());
    }

    #[test]
    fn data_directory_rejects_cwd_fallback_and_prefers_portable_marker() -> Result<()> {
        let root = std::env::temp_dir().join(format!("paperroom-paths-{}", std::process::id()));
        std::fs::create_dir(&root)?;
        let exe = root.join("paperroom.exe");
        let local = root.join("local");
        let adjacent = root.join("paperroom-data");
        for invalid in ["", "relative", r"C:relative", r"\rooted"] {
            assert!(workspace_directory(&exe, false, Some(PathBuf::from(invalid))).is_err());
        }
        assert!(workspace_directory(&exe, false, None).is_err());
        assert_eq!(workspace_directory(&exe, false, Some(local.clone()))?, local.join("Paperroom"));
        assert_eq!(workspace_directory(&exe, true, None)?, adjacent);
        std::fs::create_dir(&adjacent)?;
        assert_eq!(workspace_directory(&exe, false, Some(local))?, adjacent);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn native_markdown_editor_preserves_rtf_like_unicode_text() {
        unsafe {
            let module = GetModuleHandleW(null());
            let rich_edit = LoadLibraryExW(
                wide("Msftedit.dll").as_ptr(),
                null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            );
            assert!(!rich_edit.is_null());
            let controls = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_TREEVIEW_CLASSES | ICC_STANDARD_CLASSES,
            };
            assert_ne!(InitCommonControlsEx(&controls), 0);
            let host = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                400,
                300,
                null_mut(),
                null_mut(),
                module,
                null(),
            );
            assert!(!host.is_null());
            let widgets = Widgets::create(host, module).unwrap();
            let manuscript = "{\\rtf1 原稿} 📝\r\n第二行";
            assert_ne!(SetWindowTextW(widgets.body, wide(manuscript).as_ptr()), 0);
            assert_eq!(read_text(widgets.body).unwrap(), manuscript);
            assert!(!load_editor_text(widgets.body, "前段\0後段"));
            assert_eq!(read_text(widgets.body).unwrap(), manuscript);
            assert!(!load_editor_text(null_mut(), "標題"));
            SendMessageW(widgets.preview, EM_SETREADONLY, 0, 0);
            set_rtf(widgets.preview, &crate::markdown::preview("## 閱讀標題"));
            SendMessageW(widgets.preview, EM_SETREADONLY, 1, 0);
            assert!(read_text(widgets.preview).unwrap().contains("閱讀標題"));
            assert_eq!(read_text(widgets.body).unwrap(), manuscript);
            assert_ne!(DestroyWindow(host), 0);
        }
    }
}
