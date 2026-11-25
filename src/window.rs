use crate::archive;

#[path = "typed_property_ui.rs"]
mod typed_property_ui;
#[path = "vault_edit_ui.rs"]
mod vault_edit_ui;
#[path = "vault_sync_ui.rs"]
mod vault_sync_ui;
#[path = "vault_search_ui.rs"]
mod vault_search_ui;
#[path = "navigation_ui.rs"]
mod navigation_ui;
#[path = "synced_blocks_ui.rs"]
mod synced_blocks_ui;
#[path = "journal_ui.rs"]
mod journal_ui;
#[path = "zotero_ui.rs"]
mod zotero_ui;
#[path = "mind_map_ui.rs"]
mod mind_map_ui;
#[path = "task_ui.rs"]
mod task_ui;
#[path = "note_tools_ui.rs"]
mod note_tools_ui;
#[path = "duplicate_note_ui.rs"]
mod duplicate_note_ui;
#[path = "note_history_ui.rs"]
mod note_history_ui;
#[path = "board_objects_ui.rs"]
mod board_objects_ui;
#[path = "board_image_ui.rs"]
mod board_image_ui;
#[path = "board_shape_input.rs"]
mod board_shape_input;
#[path = "board_shape_tag_ui.rs"]
mod board_shape_tag_ui;
use crate::store::{
    BoardEdge, DEFAULT_BOARD_ID, MAX_KEY_BYTES, MAX_NOTE_BYTES, MAX_TITLE_CHARS, MAX_VALUE_BYTES,
    Note, Notebook, Result, Row, View, key_checked,
};
use crate::store::board_objects::BoardItem;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows_sys::Win32::System::LibraryLoader::*;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW,
    SHGetPathFromIDListEx, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[path = "notion_preview_ui.rs"]
mod notion_preview_ui;
#[path = "workspace_ui.rs"]
mod workspace_ui;
use workspace_ui::WorkspaceView;

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
    board_image_thumbs: HashMap<i64, Option<board_image_ui::BoardImageThumbnail>>,
    edges: Vec<BoardEdge>,
    groups: Vec<crate::store::BoardGroup>,
    objects: Vec<crate::store::board_objects::Object>,
    mind_map_nodes: Vec<crate::store::mind_map::MindNode>,
    board_shapes: Vec<crate::store::board_shapes::Shape>,
    board_shape_drag: Option<board_shape_tag_ui::BoardShapeDrag>,
    object_drag: Option<(i64, i32, i32)>,
    board_selected: HashSet<BoardItem>,
    board_batch_drag: Option<board_objects_ui::BoardBatchDrag>,
    board_drop_target: Option<board_objects_ui::BoardDropTarget>,
    collapsed_board_groups: HashSet<(i64, i64)>,
    note_find_query: String,
    block_returns: Vec<navigation_ui::BlockReturn>,
    workspace: Option<WorkspaceView>,
    board_selection: Option<(i64, String)>,
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

unsafe fn copy_unicode_text(window: HWND, text: &str) -> Result<()> {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    let native_text = text.replace("\r\n", "\n").replace(['\r', '\n'], "\r\n");
    let units: Vec<u16> = native_text.encode_utf16().chain([0]).collect();
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * size_of::<u16>());
        if memory.is_null() {
            return Err("無法配置剪貼簿記憶體。".into());
        }
        let data = GlobalLock(memory);
        if data.is_null() {
            GlobalFree(memory);
            return Err("無法寫入剪貼簿記憶體。".into());
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), data.cast::<u16>(), units.len());
        GlobalUnlock(memory);
        if OpenClipboard(window) == 0 {
            GlobalFree(memory);
            return Err("剪貼簿目前無法開啟，請重試。".into());
        }
        let copied = EmptyClipboard() != 0 && !SetClipboardData(CF_UNICODETEXT, memory).is_null();
        CloseClipboard();
        if !copied {
            GlobalFree(memory);
            return Err("無法將所選文字複製到剪貼簿。".into());
        }
        Ok(())
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
    match std::fs::metadata(&adjacent) {
        Ok(metadata) if metadata.is_dir() => return Ok(adjacent),
        Ok(_) => return Err("程式旁的 paperroom-data 必須是資料夾。紙間沒有切換到其他資料位置。".into()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        Err(_) if portable => return Ok(adjacent),
        Err(_) => {}
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

fn init_tree_control() -> Result<()> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_TREEVIEW_CLASSES,
    };
    if unsafe { InitCommonControlsEx(&controls) } == 0 {
        return Err("Windows 無法註冊筆記樹狀控制項（SysTreeView32）。".into());
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
        init_tree_control()?;
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
        // Native editor initialization must succeed before writing workspace state.
        std::fs::create_dir_all(&dir)?;
        let notebook = Notebook::open(&dir.join("notes.sqlite3"), true)?;
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
                    self.cancel_workspace_drag();
                    self.extras.board = false;
                    self.extras.workspace = None;
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
                    self.cancel_workspace_drag();
                    self.current = note;
                    self.extras.board = false;
                    self.extras.workspace = None;
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
            if !self.extras.board || self.extras.workspace.is_some() {
                self.cancel_board_shape_drag();
            }
            if (!self.extras.board || self.extras.workspace.is_some())
                && self.extras.board_drop_target.is_some()
            {
                self.cancel_board_selection();
                self.extras.board_drop_target = None;
            }
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
            let workspace = self.extras.workspace.as_ref().filter(|_| board);
            label(w.links, match workspace {
                Some(WorkspaceView::Kanban(_)) => "看板操作",
                Some(WorkspaceView::Canvas(_)) => "Canvas 操作",
                Some(WorkspaceView::Pdf(_)) => "PDF 操作",
                Some(WorkspaceView::Graph(_)) => "關聯圖操作",
                None if board => "白板操作",
                None => "連結",
            });
            if let Some(view) = workspace {
                label(w.crumb, view.title());
                label(w.status, view.help());
            } else if board {
                label(w.crumb, &self.board_path_label());
                label(w.status, "拖曳調整位置　Ctrl 點選多個物件　雙擊開啟　右鍵更多操作");
            } else if let Some(note) = note {
                let title_loaded = load_editor_text(w.title, &note.row.title);
                SendMessageW(w.title, EM_SETREADONLY, usize::from(!editing), 0);
                if editing {
                    let body = note.body.replace("\r\n", "\n")
                        .replace('\r', "\n").replace('\n', "\r\n");
                    self.editor_ready = load_editor_text(w.body, &body) && title_loaded;
                    EnableWindow(w.title, i32::from(self.editor_ready));
                    EnableWindow(w.body, i32::from(self.editor_ready));
                    SendMessageW(w.body, EM_EMPTYUNDOBUFFER, 0, 0);
                } else {
                    SendMessageW(w.preview, EM_SETREADONLY, 0, 0);
                    let projection = (active && note.body.contains("![[")).then(|| {
                        self.notebook.project_embeds(note.row.id, &note.body)
                            .unwrap_or_else(|_| format!("{}\n\n無法更新嵌入。請重開這篇筆記。", note.body))
                    });
                    set_rtf(w.preview, &crate::markdown::preview(projection.as_deref().unwrap_or(&note.body)));
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
                // Renaming or closing an unchanged imported note must not rewrite its
                // original Markdown line endings.
                let body = if body == note.body.replace("\r\n", "\n").replace('\r', "\n") {
                    note.body.clone()
                } else {
                    body
                };
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
                app.cancel_workspace_drag();
                app.view = view;
                app.current = None;
                app.extras.board = false;
                app.extras.workspace = None;
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
                (6, "匯入 Obsidian Markdown 資料夾…"),
                (7, "開啟今日筆記"),
                (8, "檢視文字屬性表格…"),
                (9, "匯入 CSV 表格…"),
                (10, "資料集合與看板…"),
                (11, "匯入 JSON Canvas…"),
                (12, "開啟已匯入的 Canvas…"),
                (13, "搜尋 PDF 標註…"),
                (14, "屬性型別…"),
                (15, "匯出 Obsidian Markdown ZIP…"),
                (16, "編輯 Vault Markdown 原檔…"),
                (17, "匯出筆記與附件 ZIP…"),
                (18, "匯入紙間筆記與附件 ZIP…"),
                (19, "預覽筆記庫批次同步…"),
                (20, "匯入 Markdown ZIP（相對附件）…"),
                (21, "Vault 批次寫回磁碟…"),
                (22, "搜尋 Vault 來源…"),
                (23, "命令面板…"),
                (24, "快速切換筆記…"),
                (25, "目前筆記的大綱…"),
                (26, "搜尋 PDF 摘錄卡…"),
                (27, "更新 Vault 來源附件…"),
                (28, "匯入 Zotero CSL JSON 書目…"),
                (29, "匯出 Zotero 原始 CSL JSON…"),
                (30, "匯出工作區 HTML ZIP…"),
                (31, "匯入 JSON Canvas ZIP…"),
                (32, "預覽 Notion ZIP 關聯（唯讀）…"),
                (33, "關於紙間／第三方補充通知…"),
            ];
            let action = popup(self.hwnd, self.widgets.files, None, &items);
            if action == 33 {
                let notices = crate::about_notices::body();
                if let Err(error) = crate::dialog::show_text(self.hwnd, "第三方補充通知", &notices) {
                    self.show_error(error);
                }
                return;
            }
            if action == 32 {
                // The preview must not flush an open note or invoke any workspace mutation.
                if let Err(error) = notion_preview_ui::run(self.hwnd) {
                    self.show_error(error);
                }
                return;
            }
            self.mutate(|app| {
                let is_notion_zip = action == 2;
                let action = match action {
                    1 => ("Markdown 筆記\0*.md;*.markdown\0\0", "md", None),
                    2 => ("Notion Markdown ZIP\0*.zip\0\0", "zip", None),
                    20 => ("Markdown ZIP\0*.zip\0\0", "zip", None),
                    3 => ("Markdown 筆記\0*.md\0\0", "md", Some("note.md")),
                    6 => return app.import_vault(),
                    7 => {
                        let mut today: SYSTEMTIME = std::mem::zeroed();
                        GetLocalTime(&mut today);
                        let id = app.notebook.open_daily(today.wYear, today.wMonth, today.wDay)?;
                        app.open_import(id, "已開啟今日筆記");
                        return Ok(());
                    }
                    8 => return app.property_table(),
                    9 => ("CSV 表格\0*.csv\0\0", "csv", None),
                    10 => return app.collection_picker(),
                    11 => ("JSON Canvas\0*.canvas\0\0", "canvas", None),
                    12 => return app.open_canvas(None),
                    13 => return app.search_pdf_annotations_ui(),
                    14 => return app.typed_property_menu(),
                    15 => return app.export_vault_zip_ui(),
                    16 => return app.edit_vault_markdown(),
                    17 | 18 => return app.note_bundle_dialog(action == 17),
                    19 => return app.sync_vault_incoming(),
                    21 => return app.sync_vault_outgoing(),
                    22 => return app.vault_source_picker(),
                    23 => return app.command_palette(),
                    24 => return app.quick_switcher(),
                    25 => return app.note_outline(),
                    26 => return app.search_pdf_excerpt_cards_ui(),
                    27 => return app.refresh_vault_attachment_ui(),
                    28 => return app.import_zotero_csl_ui(),
                    29 => return app.export_zotero_csl_ui(),
                    31 => return app.import_canvas_bundle_ui(),
                    30 => {
                        if let Some(path) = file_dialog(app.hwnd, "HTML ZIP\0*.zip\0\0", "zip", Some("paperroom-html.zip"))? {
                            let report = app.notebook.export_html_workspace(&path)?;
                            label(app.widgets.status, &format!("已匯出 {} 篇筆記和 {} 個附件。", report.pages, report.attachments));
                        }
                        return Ok(());
                    }
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
                        "zip" if is_notion_zip => {
                            let (imported, media, csv) = archive::notion_archive_with_media(&path)?;
                            let mut report = media.report(imported.pages.len(), imported.skipped);
                            report.push_str("\r\n\r\n");
                            report.push_str(&csv.report());
                            let status = format!(
                                "已匯入 {} 篇 Markdown、{} 筆附件關聯及 {} 份原始 CSV。",
                                imported.pages.len(), media.count(), csv.count(),
                            );
                            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Notion");
                            let id = app.notebook.import_notion_with_media(
                                &format!("匯入：{name}"), imported.pages, media, csv,
                            )?;
                            app.open_import(id, &status);
                            crate::dialog::show_text(app.hwnd, "匯入報告", &report)?;
                        }
                        "zip" => {
                            let (imported, media) = archive::archive_with_media(&path)?;
                            if let Some(notice) = imported.markdown_zip_import_notice()
                                && message(
                                    app.hwnd, "Markdown ZIP 匯入範圍", notice,
                                    MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING,
                                ) != IDYES
                            {
                                return Ok(());
                            }
                            let report = media.report(imported.pages.len(), imported.skipped);
                            let status = format!("已匯入 {} 篇 Markdown 與 {} 筆附件關聯。", imported.pages.len(), media.count());
                            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Markdown ZIP");
                            let id = app.notebook.import_markdown_with_media(
                                &format!("匯入：{name}"), imported.pages, media,
                            )?;
                            app.open_import(id, &status);
                            crate::dialog::show_text(app.hwnd, "匯入報告", &report)?;
                        }
                        "canvas" => app.open_canvas(Some(&path))?,
                        "csv" => {
                            let name = path.file_stem().and_then(|part| part.to_str()).unwrap_or("CSV");
                            let (root, count) = app.notebook.import_csv_file(&format!("匯入：{name}"), &path)?;
                            app.open_import(root, &format!("已從 CSV 匯入 {count} 列為筆記與文字屬性"));
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

    unsafe fn note_bundle_dialog(&mut self, export: bool) -> Result<()> {
        unsafe {
            let bundle_filter = "紙間筆記與附件 ZIP\0*.zip\0\0";
            if export {
                let id = self.current.as_ref().ok_or("請先開啟要匯出的筆記。")?.row.id;
                if let Some(path) = file_dialog(self.hwnd, bundle_filter, "zip", Some("paperroom-note.zip"))? {
                    let count = self.notebook.export_note_bundle(id, &path)?;
                    label(self.widgets.status, &format!("已匯出筆記及 {count} 個附件。"));
                }
            } else if let Some(path) = file_dialog(self.hwnd, bundle_filter, "zip", None)? {
                let (id, count) = self.notebook.import_note_bundle(&path)?;
                self.open_import(id, &format!("已匯入新筆記及 {count} 個附件。"));
            }
            Ok(())
        }
    }

    unsafe fn import_vault(&mut self) -> Result<()> {
        unsafe {
            let Some(path) = folder_dialog(self.hwnd)? else { return Ok(()); };
            let vault = crate::vault::read(&path)?;
            let media = crate::store::migration_media::plan(&path, &vault)?;
            let report = media.report(vault.pages.len(), vault.skipped.saturating_sub(media.file_count()));
            let status = format!("已匯入 {} 篇 Markdown 與 {} 筆附件關聯。", vault.pages.len(), media.count());
            let name = path.file_name().and_then(|part| part.to_str()).unwrap_or("Obsidian");
            let root = self.notebook.import_vault_with_media(&format!("匯入：{name}"), vault.pages, media)?;
            self.open_import(root, &status);
            crate::dialog::show_text(self.hwnd, "匯入報告", &report)?;
            Ok(())
        }
    }

    unsafe fn property_table(&self) -> Result<()> {
        unsafe {
            let owner = self.hwnd;
            let Some(key) = crate::dialog::prompt(owner, "文字屬性表格", "屬性名稱", "", MAX_KEY_BYTES)? else {
                return Ok(());
            };
            let Some(query) = crate::dialog::prompt(
                owner, "文字屬性表格", "值包含的文字（區分大小寫；留空顯示全部）", "", 180,
            )? else { return Ok(()); };
            let key = key_checked(&key)?;
            let rows = self.notebook.property_rows(key, &query)?;
            let mut contents = format!("標題\t{key}\r\n");
            for (row, value) in &rows {
                use std::fmt::Write;
                let value = value.replace(['\t', '\r', '\n'], " ");
                writeln!(contents, "{}\t{value}\r", row.title.replace(['\t', '\r', '\n'], " "))?;
            }
            if rows.is_empty() {
                contents.push_str("沒有符合的筆記。");
            } else if rows.len() == 1000 {
                contents.push_str("\r\n已達 1,000 筆顯示上限。請縮小篩選範圍。");
            }
            crate::dialog::show_text(owner, "文字屬性表格", &contents)
        }
    }

    unsafe fn note_properties(&self, id: i64, action: usize) -> Result<()> {
        unsafe {
            let owner = self.hwnd;
            let existing = self.notebook.properties(id)?;
            if action == 6 {
                let mut contents = String::new();
                for (key, value) in existing {
                    use std::fmt::Write;
                    writeln!(contents, "{key}：{value}\r")?;
                }
                if contents.is_empty() {
                    contents.push_str("這篇筆記尚無文字屬性。");
                }
                return crate::dialog::show_text(owner, "筆記的文字屬性", &contents);
            }
            let Some(key) = crate::dialog::prompt(owner, "文字屬性", "屬性名稱", "", MAX_KEY_BYTES)? else {
                return Ok(());
            };
            let key = key_checked(&key)?;
            let expected = existing.iter().find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str());
            if action == 5 {
                return self.notebook.compare_set_property(id, key, expected, None);
            }
            let hint = self.notebook.property_type(key)?.map(|kind| kind.hint()).unwrap_or("屬性值");
            if let Some(value) = crate::dialog::prompt_multiline(
                owner, "設定文字屬性", hint, expected.unwrap_or(""), MAX_VALUE_BYTES,
            )? {
                if let Err(error) = self.notebook.compare_set_property(id, key, expected, Some(&value)) {
                    crate::dialog::show_text(owner, "未儲存的屬性值（可複製）", &value)?;
                    return Err(error);
                }
            }
            Ok(())
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
                let mut items = vec![
                    (1, star), (2, "移到…"), (3, "移到垃圾桶"),
                    (4, "設定文字屬性…"), (5, "移除文字屬性…"), (6, "檢視文字屬性…"),
                    (7, "筆記關聯…"), (8, "附件與 PDF 摘錄…"),
                ];
                let is_excerpt = app.notebook.is_pdf_excerpt(id)?;
                if is_excerpt {
                    items.push((9, "開啟 PDF 摘錄來源"));
                }
                items.extend([(20, "尋找／取代…"), (21, "字數統計…"), (22, "檢視與還原版本…")]);
                if is_excerpt {
                    items.push((23, "復原已移除的 PDF 標註…"));
                }
                items.push((24, "複製筆記與內容…"));
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
                    action @ 4..=6 => app.note_properties(id, action)?,
                    7 => app.relation_menu(id)?,
                    8 => app.attachments_menu(id)?,
                    9 => app.open_pdf_excerpt_source(id)?,
                    20 => app.find_replace_note_ui()?,
                    21 => app.note_word_count_ui()?,
                    22 => app.note_history_ui(id)?,
                    23 => app.restore_pdf_excerpt_ui(id)?,
                    24 => app.duplicate_note_ui(id)?,
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
            if self.navigation_shortcut(msg) {
                return true;
            }
            if msg.message == WM_CHAR
                && msg.wParam == b'/' as usize
                && msg.hwnd == self.widgets.body
                && self.editing
                && self.caret_line().1 == 0
            {
                return self.block_menu(true);
            }
            if msg.message == WM_KEYDOWN && msg.wParam == VK_ESCAPE as usize
                && (self.cancel_workspace_drag() || self.cancel_board_shape_drag() || self.cancel_board_selection())
            {
                return true;
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

unsafe fn folder_dialog(owner: HWND) -> Result<Option<PathBuf>> {
    unsafe {
        use std::os::windows::ffi::OsStringExt;
        let status = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        if status < 0 {
            return Err(format!("無法開啟資料夾選擇器（{status:#x}）。").into());
        }
        let title = wide("選擇要匯入的 Obsidian Vault 資料夾");
        let mut display = [0u16; 260];
        let options = BROWSEINFOW {
            hwndOwner: owner,
            pidlRoot: null_mut(),
            pszDisplayName: display.as_mut_ptr(),
            lpszTitle: title.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            lpfn: None,
            lParam: 0,
            iImage: 0,
        };
        let selected = SHBrowseForFolderW(&options);
        let result = if selected.is_null() {
            Ok(None)
        } else {
            let mut path = vec![0u16; 32_768];
            let ok = SHGetPathFromIDListEx(selected, path.as_mut_ptr(), path.len() as u32, 0);
            CoTaskMemFree(selected.cast());
            match path.iter().position(|unit| *unit == 0) {
                Some(end) if ok != 0 && end != 0 => Ok(Some(PathBuf::from(
                    std::ffi::OsString::from_wide(&path[..end]),
                ))),
                _ => Err("請選擇實際存在的檔案系統資料夾。".into()),
            }
        };
        CoUninitialize();
        result
    }
}

impl App {
    unsafe fn links_menu(&mut self) {
        use crate::markdown::AnchorSyntax;
        unsafe {
            if self.extras.board {
                self.mutate(|app| {
                    if app.extras.workspace.is_some() { app.workspace_menu() }
                    else { app.board_menu() }
                });
                return;
            }
            self.mutate(|app| {
                let Some(note) = &app.current else {
                    return Ok(());
                };
                let source = note.row.id;
                let (outgoing, incoming) = app.notebook.related(note)?;
                let mut labels = vec![(1, "插入筆記連結…".to_owned())];
                let links = app.notebook.vault_note_links(note)?;
                let handled = links.iter().map(|target| target.note_id).collect::<HashSet<_>>();
                let mut related = links.into_iter().map(|link| {
                    let anchor = link.anchor.as_deref().unwrap_or("");
                    let action = if anchor.starts_with("#^") { "預覽" } else { "前往" };
                    let label = format!("{action}　{} {anchor}", link.label);
                    (link.note_id, label, link.anchor, link.anchor_syntax)
                }).collect::<Vec<_>>();
                related.extend(outgoing.iter().filter(|row| !handled.contains(&row.id))
                    .map(|row| (row.id, format!("前往　{}", row.title), None, AnchorSyntax::Wiki)));
                related.extend(incoming.iter().map(|row| (row.id, format!("反向連結　{}", row.title), None, AnchorSyntax::Wiki)));
                for (index, (_, title, _, _)) in related.iter().enumerate() {
                    labels.push((index + 2, title.clone()));
                }
                let graph = related.len() + 2;
                labels.push((graph, "開啟筆記關聯圖…".to_owned()));
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
                            app.replace_selection(&format!("[[paperroom:note:{}|{name}]]", target.id));
                        }
                    }
                    index if index == graph => app.open_note_graph(source)?,
                    index => {
                        if let Some((id, _, heading, syntax)) = related.get(index - 2) {
                            app.open_anchored_link(*id, heading.as_deref(), *syntax)?;
                        }
                    }
                }
                Ok(())
            });
        }
    }

    fn board_id(&self) -> i64 {
        self.extras.board_selection.as_ref().map_or(DEFAULT_BOARD_ID, |(id, _)| *id)
    }

    unsafe fn board_menu(&mut self) -> Result<()> {
        unsafe {
            let anchor = self.widgets.links;
            match popup(self.hwnd, anchor, None, &[
                (1, "放入筆記…"), (2, "切換白板…"), (3, "新增白板…"), (4, "新增分區…"),
                (5, "重新命名分區…"), (6, "整組移動分區…"), (7, "檢視分區來源…"),
                (8, "分區釘選 PDF 標註…"),
                (9, "開啟摘錄卡的 PDF 來源…"),
                (10, "子白板與文字…"),
                (11, "搜尋並放入 PDF 摘錄卡…"),
                (12, "移動選取的物件…"), (13, "收合或展開分區…"),
                (14, "將選取卡片放到另一張白板…"),
                (15, "將選取卡片搬移到另一張白板…"),
                (16, "指定跨板拖曳目的地…"),
                (17, "卡片資料庫…"),
                (18, "貼上剪貼簿圖片"), (19, "貼上圖片檔案"),
                (20, "匯入本機圖片…"),
                (21, "匯出研究白板與附件…"), (22, "匯入研究白板與附件…"),
                (23, "對齊或等距排列…"), (24, "復原最近一次排列"),
                (25, "新增心智圖根節點…"), (26, "心智圖節點…"),
                (27, "白板形狀…"), (28, "白板標籤…"),
            ]) {
                1 => {
                    let notes = self.notebook.rows(View::All, "")?;
                    let choices = notes.iter().enumerate()
                        .map(|(index, row)| (index + 1, row.title.as_str()))
                        .collect::<Vec<_>>();
                    let action = popup(self.hwnd, anchor, None, &choices);
                    if let Some(row) = action.checked_sub(1).and_then(|index| notes.get(index)) {
                        self.notebook.add_card_on(self.board_id(), row.id)?;
                        self.refresh_board()?;
                    }
                }
                2 => {
                    let boards = self.notebook.list_boards()?;
                    let choices = boards.iter().enumerate()
                        .map(|(index, board)| (index + 1, board.title.as_str()))
                        .collect::<Vec<_>>();
                    let action = popup(self.hwnd, anchor, None, &choices);
                    if let Some(board) = action.checked_sub(1).and_then(|index| boards.get(index)) {
                        self.select_board(board.id, board.title.clone())?;
                    }
                }
                3 => {
                    if let Some(title) = crate::dialog::prompt(
                        self.hwnd, "新增白板", "白板名稱", "", MAX_TITLE_CHARS,
                    )? {
                        let id = self.notebook.create_board(&title)?;
                        self.select_board(id, crate::store::clean_title(&title))?;
                    }
                }
                4 => {
                    if let Some(title) = crate::dialog::prompt(self.hwnd, "新增分區", "分區名稱", "", 80)? {
                        self.notebook.create_board_group(self.board_id(), &title)?;
                        self.refresh_board()?;
                        label(self.widgets.status, "分區已新增。右鍵點選卡片即可分區。");
                    }
                }
                action @ (5..=8) => self.board_group_menu(action)?,
                9 => self.open_board_pdf_excerpt_source()?,
                10 => self.board_object_menu()?,
                11 => self.add_existing_pdf_excerpt_ui()?,
                12 => self.board_selection_menu()?,
                13 => self.collapse_board_group_menu()?,
                14 => self.reuse_selected_board_cards_menu()?,
                15 => self.move_selected_board_cards_menu()?,
                16 => self.arm_board_drop_target()?,
                17 => self.board_card_library_ui()?,
                18 => self.paste_board_image_pixels()?,
                19 => self.paste_board_image_files()?,
                20 => self.import_board_image_file()?,
                21 => self.export_board_bundle_ui()?,
                22 => self.import_board_bundle_ui()?,
                23 => self.board_layout_menu()?,
                24 => self.undo_board_layout_ui()?,
                25 => self.add_mind_map_root_ui()?,
                26 => self.mind_map_picker_ui()?,
                27 => self.board_shapes_menu()?,
                28 => self.board_tags_menu()?,
                _ => {}
            }
            Ok(())
        }
    }

    unsafe fn export_board_bundle_ui(&self) -> Result<()> {
        unsafe {
            let Some(path) = file_dialog(
                self.hwnd, "研究白板 ZIP\0*.zip\0\0", "zip", Some("研究白板.zip"),
            )? else { return Ok(()); };
            self.notebook.export_board_bundle(self.board_id(), &path)?;
            label(self.widgets.status, "已匯出含圖片附件的研究白板 ZIP。");
            Ok(())
        }
    }

    unsafe fn import_board_bundle_ui(&mut self) -> Result<()> {
        unsafe {
            let Some(path) = file_dialog(self.hwnd, "研究白板 ZIP\0*.zip\0\0", "zip", None)?
                else { return Ok(()); };
            let default = path.file_stem().and_then(|s| s.to_str())
                .map(crate::store::clean_title).unwrap_or_else(|| "研究白板".to_owned());
            let Some(title) = crate::dialog::prompt(
                self.hwnd, "匯入研究白板", "請為匯入的新白板命名。", &default, MAX_TITLE_CHARS,
            )? else { return Ok(()); };
            let id = self.notebook.import_board_bundle(&path, &title)?;
            self.select_board(id, crate::store::clean_title(&title))?;
            label(self.widgets.status, "已放入新白板，圖片已保存至工作區。");
            Ok(())
        }
    }

    unsafe fn select_board(&mut self, id: i64, title: String) -> Result<()> {
        unsafe {
            self.cancel_board_shape_drag();
            self.cancel_board_selection();
            let previous = self.extras.board_selection.replace((id, title));
            if let Err(error) = self.refresh_board() {
                self.extras.board_selection = previous;
                return Err(error);
            }
            self.extras.board_drop_target = None;
            self.extras.board_selected.clear();
            self.extras.scroll = 0;
            self.present();
            Ok(())
        }
    }

    unsafe fn refresh_board(&mut self) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let cards = self.notebook.cards_on(board)?;
            let edges = self.notebook.edges_on(board)?;
            let groups = self.notebook.groups_on(board)?;
            let objects = self.notebook.objects_on(board)?;
            let mind_map_nodes = self.notebook.mind_nodes_on(board)?;
            let board_shapes = self.notebook.board_shapes(board)?;
            self.extras.cards = cards;
            self.extras.edges = edges;
            self.extras.groups = groups;
            self.extras.objects = objects;
            self.extras.mind_map_nodes = mind_map_nodes;
            self.extras.board_shapes = board_shapes;
            let visible: HashSet<_> = self.extras.cards.iter()
                .filter(|card| !self.board_card_collapsed(card.id)).map(|card| BoardItem::Card(card.id))
                .chain(self.extras.objects.iter().map(|object| BoardItem::Object(object.id))).collect();
            self.extras.board_selected.retain(|item| visible.contains(item));
            let end = self.board_scroll_end();
            self.extras.scroll = self.extras.scroll.clamp(0, end);
            self.refresh_board_image_cache();
            InvalidateRect(self.hwnd, null(), 1);
            Ok(())
        }
    }

    fn board_scroll_end(&self) -> i32 {
        self.extras.cards.iter().map(|card| card.y + (card.size().height - 114).max(0))
            .chain(self.extras.objects.iter().map(|object| object.y))
            .chain(self.extras.board_shapes.iter().map(|shape| shape.rect.y + (shape.rect.height - 114).max(0)))
            .max().unwrap_or(0)
    }

    unsafe fn board_toggle(&mut self) {
        unsafe {
            self.mutate(|app| {
                app.cancel_workspace_drag();
                app.cancel_board_shape_drag();
                app.cancel_board_selection();
                app.extras.board_drop_target = None;
                app.extras.workspace = None;
                app.extras.board = !app.extras.board;
                app.extras.board_selected.clear();
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
                .filter(|card| !self.board_card_collapsed(card.id))
                .find(|card| {
                    let left = area.left + self.scaled(card.x);
                    let top = area.top + self.scaled(card.y - self.extras.scroll);
                    let size = card.size();
                    (left..left + self.scaled(size.width)).contains(&x)
                        && (top..top + self.scaled(size.height)).contains(&y)
                })
                .map(|card| card.id)
        }
    }

    unsafe fn board_input(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) {
        unsafe {
            if !self.extras.board {
                return;
            }
            if self.extras.workspace.is_some() {
                self.workspace_input(msg, wp, lp);
                return;
            }
            let x = lp as u16 as i16 as i32;
            let y = (lp >> 16) as u16 as i16 as i32;
            if self.board_shape_drag_input(msg, x, y) {
                return;
            }
            if self.board_batch_input(msg, wp, x, y) {
                return;
            }
            if self.board_object_input(msg, x, y) {
                return;
            }
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
                    } else if let Some(shape) = self.board_shape_at(x, y) {
                        self.begin_board_shape_drag(shape, x, y);
                    }
                }
                WM_MOUSEMOVE => {
                    if let Some((id, dx, dy)) = self.extras.dragging {
                        let area = self.board_area();
                        let dpi = self.dpi as i32;
                        let width = self.extras.cards.iter().find(|card| card.id == id)
                            .map_or(232, |card| card.size().width);
                        let max_x = (area.right - area.left) * 96 / dpi - width;
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
                        if let Some(card) = self.extras.cards.iter().find(|c| c.id == id) {
                            if let Err(error) = self.notebook.move_card_on(self.board_id(), id, card.x, card.y) {
                                let _ = self.refresh_board();
                                self.show_error(error);
                            } else {
                                self.refresh_board_image_cache();
                                InvalidateRect(self.hwnd, null(), 1);
                            }
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
                        if let Err(error) = self.board_card_menu(id, pt) {
                            self.show_error(error);
                        }
                    } else if let Some(shape) = self.board_shape_at(x, y) {
                        let mut pt = POINT { x, y };
                        ClientToScreen(self.hwnd, &mut pt);
                        if let Err(error) = self.board_shape_context_menu(shape, pt) {
                            self.show_error(error);
                        }
                    }
                }
                WM_MOUSEWHEEL => {
                    let mut cursor = POINT { x, y };
                    ScreenToClient(self.hwnd, &mut cursor);
                    let area = self.board_area();
                    if cursor.x >= area.left && cursor.x < area.right {
                        let delta = (wp >> 16) as u16 as i16 as i32;
                        let max = self.board_scroll_end();
                        self.extras.scroll = (self.extras.scroll - delta).clamp(0, max);
                        self.refresh_board_image_cache();
                        InvalidateRect(self.hwnd, &area, 1);
                    }
                }
                _ => {}
            }
        }
    }

    unsafe fn board_card_menu(&mut self, id: i64, point: POINT) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let mut items = vec![
                (1, "移出白板"), (2, "與另一張卡片連線…"), (3, "移除連線…"), (4, "設定分區…"),
            ];
            if self.notebook.is_pdf_excerpt(id)? {
                items.push((5, "開啟 PDF 摘錄來源"));
            }
            if self.extras.cards.iter().any(|card| card.id == id && card.image.is_some()) {
                items.push((6, "調整圖片卡大小…"));
            }
            let action = popup(self.hwnd, null_mut(), Some(point), &items);
            if action == 6 {
                return self.resize_board_image_ui(id);
            }
            if action == 5 {
                return self.open_pdf_excerpt_source(id);
            }
            if action == 1 {
                self.notebook.remove_card_on(board, id)?;
                return self.refresh_board();
            }
            if action == 4 {
                let expected = self.extras.groups.iter().find(|group| group.members.contains(&id)).map(|g| g.id);
                let mut choices = vec![(1, "不分區")];
                choices.extend(self.extras.groups.iter().enumerate().map(|(i, group)| (i+2, group.title.as_str())));
                let selection = popup(self.hwnd, null_mut(), Some(point), &choices);
                match selection {
                    0 => {}
                    1 => self.notebook.set_board_group(board, id, expected, None)?,
                    n => if let Some(group) = self.extras.groups.get(n-2) {
                        self.notebook.set_board_group(board, id, expected, Some(group.id))?;
                    },
                }
                return self.refresh_board();
            }
            if !matches!(action, 2 | 3) {
                return Ok(());
            }
            let is_connected = |other| self.extras.edges.iter().any(|edge| {
                (edge.from == id && edge.to == other) || (edge.to == id && edge.from == other)
            });
            let candidates = self.extras.cards.iter().filter(|card| {
                card.id != id && is_connected(card.id) == (action == 3)
            }).collect::<Vec<_>>();
            if candidates.is_empty() {
                label(self.widgets.status, "沒有符合的連線卡片。");
                return Ok(());
            }
            let choices = candidates.iter().enumerate()
                .map(|(index, card)| (index + 1, card.title.as_str()))
                .collect::<Vec<_>>();
            let action_id = popup(self.hwnd, null_mut(), Some(point), &choices);
            if let Some(card) = action_id.checked_sub(1).and_then(|index| candidates.get(index)) {
                if action == 2 {
                    self.notebook.add_edge_on(board, id, card.id)?;
                } else {
                    self.notebook.remove_edge_on(board, id, card.id)?;
                }
                self.refresh_board()?;
            }
            Ok(())
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
            self.paint_board_drop_target(dc, area);
            let saved = SaveDC(dc);
            IntersectClipRect(dc, area.left, area.top, area.right, area.bottom);
            if let Some(view) = &self.extras.workspace {
                self.paint_workspace(dc, area, view);
                RestoreDC(dc, saved);
                return;
            }
            fill(dc, &area, color(246, 248, 243));
            let step = self.scaled(22).max(1) as usize;
            for y in (area.top..area.bottom).step_by(step) {
                for x in (area.left..area.right).step_by(step) {
                    SetPixelV(dc, x, y, LINE);
                }
            }
            if self.extras.cards.is_empty() && self.extras.objects.is_empty() && self.extras.board_shapes.is_empty() {
                let mut guide = RECT {
                    left: area.left + self.scaled(24),
                    top: area.top + self.scaled(24),
                    ..area
                };
                draw_text(
                    dc,
                    self.fonts[4],
                    SOFT,
                    "從「白板操作」選擇「放入筆記」。",
                    &mut guide,
                    DT_SINGLELINE | DT_END_ELLIPSIS,
                );
            }
            for group in &self.extras.groups {
                let mut bounds = None::<RECT>;
                for card in self.extras.cards.iter().filter(|card| group.members.contains(&card.id)) {
                    let left = area.left + self.scaled(card.x);
                    let top = area.top + self.scaled(card.y - self.extras.scroll);
                    let size = card.size();
                    let rect = RECT { left, top, right: left + self.scaled(size.width), bottom: top + self.scaled(size.height) };
                    if let Some(r) = &mut bounds {
                        r.left = r.left.min(rect.left);
                        r.top = r.top.min(rect.top);
                        r.right = r.right.max(rect.right);
                        r.bottom = r.bottom.max(rect.bottom);
                    } else {
                        bounds = Some(rect);
                    }
                }
                if let Some(mut rect) = bounds {
                    rect.left -= self.scaled(5);
                    rect.right += self.scaled(5);
                    rect.top -= self.scaled(21);
                    rect.bottom += self.scaled(5);
                    let collapsed = self.extras.collapsed_board_groups.contains(&(self.board_id(), group.id));
                    if collapsed {
                        rect.bottom = rect.top + self.scaled(21);
                        if rect.bottom > area.top && rect.top < area.top {
                            rect.top = area.top;
                            rect.bottom = rect.top + self.scaled(21);
                        }
                        fill(dc, &rect, MINT);
                    }
                    let brush = CreateSolidBrush(TEAL);
                    if !brush.is_null() { FrameRect(dc, &rect, brush); DeleteObject(brush); }
                    rect.left += self.scaled(5);
                    rect.top += self.scaled(2);
                    let title = if collapsed { format!("{}（已收合）", group.title) } else { group.title.clone() };
                    draw_text(dc, self.fonts[4], TEAL, &title, &mut rect,
                              DT_SINGLELINE | DT_END_ELLIPSIS);
                }
            }
            // Shapes are deliberately below cards and existing hit targets.
            self.paint_board_shapes(dc, area);
            if !self.extras.edges.is_empty() {
                let pen = CreatePen(PS_SOLID, self.scaled(2).max(1), TEAL);
                if !pen.is_null() {
                    let prior = SelectObject(dc, pen);
                    let centers = self.extras.cards.iter().filter(|card| !self.board_card_collapsed(card.id)).map(|card| {
                        let size = card.size();
                        (card.id, (area.left + self.scaled(card.x + size.width / 2),
                                   area.top + self.scaled(card.y - self.extras.scroll + size.height / 2)))
                    }).collect::<HashMap<_, _>>();
                    for edge in &self.extras.edges {
                        if let (Some(&(from_x, from_y)), Some(&(to_x, to_y))) =
                            (centers.get(&edge.from), centers.get(&edge.to))
                        {
                            MoveToEx(dc, from_x, from_y, null_mut());
                            LineTo(dc, to_x, to_y);
                        }
                    }
                    SelectObject(dc, prior);
                    DeleteObject(pen);
                }
            }
            self.paint_mind_map_edges(dc, area);
            for card in self.extras.cards.iter().filter(|card| !self.board_card_collapsed(card.id)) {
                let left = area.left + self.scaled(card.x);
                let top = area.top + self.scaled(card.y - self.extras.scroll);
                let size = card.size();
                let rect = RECT {
                    left,
                    top,
                    right: left + self.scaled(size.width),
                    bottom: top + self.scaled(size.height),
                };
                if rect.bottom <= area.top || rect.top >= area.bottom {
                    continue;
                }
                fill(dc, &rect, PAPER);
                if self.paint_board_image(dc, rect, card) {
                    self.paint_board_item_frame(dc, rect, BoardItem::Card(card.id));
                    continue;
                }
                self.paint_board_item_frame(dc, rect, BoardItem::Card(card.id));
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
            self.paint_board_objects(dc, area);
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
                if let Ok(mut app) = state.try_borrow_mut() {
                    app.layout();
                    if app.extras.board && app.extras.workspace.is_none() {
                        app.refresh_board_image_cache();
                    }
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
                    if app.extras.board && app.extras.workspace.is_none() {
                        app.refresh_board_image_cache();
                    }
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
            WM_TIMER if wp == workspace_ui::PDF_TIMER => {
                if let Ok(mut app) = state.try_borrow_mut() {
                    app.workspace_timer();
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
            | WM_MOUSEWHEEL | WM_CAPTURECHANGED | WM_CANCELMODE => {
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
        std::fs::write(&adjacent, "existing data")?;
        assert!(workspace_directory(&exe, false, Some(local.clone())).is_err());
        assert!(workspace_directory(&exe, true, None).is_err());
        assert_eq!(std::fs::read_to_string(&adjacent)?, "existing data");
        assert!(!local.exists());
        std::fs::remove_file(&adjacent)?;
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
            init_tree_control().unwrap();
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
            // Probe the real standard Unicode EDIT control used for source text.
            // The mixed source separators themselves are not written to a database here;
            // UI projection, native selection coordinates, and storage are separate gates.
            let source = "前段\r\n第二行\n第三行\r選取 📝 結尾\r\n保留下一行";
            let projected = source.replace("\r\n", "\n")
                .replace('\r', "\n").replace('\n', "\r\n");
            assert_eq!(GetDlgItem(host, i32::from(BODY)), widgets.body);
            assert_eq!(GetDlgCtrlID(widgets.body), i32::from(BODY));
            let mut owner_pid = 0;
            assert_ne!(GetWindowThreadProcessId(widgets.body, std::ptr::from_mut(&mut owner_pid)), 0);
            assert_eq!(owner_pid, std::process::id());
            assert!(load_editor_text(widgets.body, &projected));
            assert_eq!(read_text(widgets.body).unwrap(), projected);

            let emoji = projected.find('📝').unwrap();
            let start = projected[..emoji].encode_utf16().count();
            let end = start + '📝'.len_utf16();
            SendMessageW(widgets.body, EM_SETSEL, start, end as isize);
            let (mut found_start, mut found_end) = (u32::MAX, u32::MAX);
            SendMessageW(widgets.body, EM_GETSEL,
                std::ptr::from_mut(&mut found_start) as usize,
                std::ptr::from_mut(&mut found_end) as isize);
            assert_eq!((found_start as usize, found_end as usize), (start, end));

            SendMessageW(widgets.body, EM_REPLACESEL, 1, wide("〔📝〕").as_ptr() as isize);
            assert_eq!(read_text(widgets.body).unwrap(), projected.replace('📝', "〔📝〕"));
            assert_ne!(DestroyWindow(host), 0);
        }
    }
}
