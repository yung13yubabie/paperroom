use super::*;
use crate::canvas_projection::{Drag, Projection};
use std::fmt::Write;

#[path = "canvas_editor_ui.rs"]
mod canvas_editor_ui;
#[path = "canvas_view_ui.rs"]
mod canvas_view_ui;
#[path = "pdf_view_ui.rs"]
mod pdf_view_ui;
#[path = "pdf_search_ui.rs"]
mod pdf_search_ui;
#[path = "note_graph_ui.rs"]
mod note_graph_ui;
#[path = "collection_view_ui.rs"]
mod collection_view_ui;
#[path = "board_group_ui.rs"]
mod board_group_ui;
pub(super) use pdf_view_ui::PDF_TIMER;

pub(super) enum WorkspaceView {
    Kanban(Kanban),
    Canvas(Canvas),
    Pdf(pdf_view_ui::PdfView),
    Graph(note_graph_ui::Graph),
}

pub(super) struct Kanban {
    id: i64,
    title: String,
    lanes: Vec<(String, Vec<Row>)>,
    column: usize,
    row: usize,
    saved_view: Option<crate::store::collection_views::CollectionView>,
    checkbox: Option<(String, std::collections::HashMap<i64, String>)>,
    week: Option<(crate::store::collection_views::CalendarDay,
        std::collections::BTreeMap<crate::store::collection_views::CalendarDay, Vec<Row>>)>,
    calendar: Option<(crate::store::collection_views::Month, std::collections::BTreeMap<usize, Vec<Row>>)>,
    timeline: Option<(crate::store::collection_views::Month, Vec<crate::store::collection_views::ScheduledRow>)>,
    timeline_drag: Option<collection_view_ui::TimelineDrag>,
}

pub(super) struct Canvas {
    id: i64,
    title: String,
    doc: crate::canvas::Document,
    bundle: Option<crate::store::canvas_bundle::CanvasBundleView>,
    projection: Cell<Option<Projection>>,
    drag: Option<Drag>,
}

impl WorkspaceView {
    pub(super) fn title(&self) -> &str {
        match self {
            Self::Kanban(view) => view.saved_view.as_ref().map_or(&view.title, |saved| &saved.name),
            Self::Canvas(view) => &view.title,
            Self::Pdf(view) => &view.title,
            Self::Graph(view) => &view.title,
        }
    }

    pub(super) fn help(&self) -> &'static str {
        match self {
            Self::Kanban(view) if view.timeline.is_some() => "拖曳橫條改期或拖可見端點調整　點選開啟筆記　Esc 取消",
            Self::Kanban(view) if view.week.is_some() => "點選有筆記的日期或使用選單開啟　可切換上一週或下一週",
            Self::Kanban(view) if view.calendar.is_some() => "點選有筆記的日期或使用選單開啟　可切換上個月或下個月",
            Self::Kanban(view) if view.is_list() => "點選資料列開啟筆記　滾輪捲動清單",
            Self::Kanban(view) if view.checkbox.is_some() => "點選右欄切換勾選　資料列會依照篩選更新",
            Self::Kanban(_) => "使用看板操作新增或移動資料列　滾輪捲動卡片",
            Self::Canvas(_) => "拖曳節點移動，拖曳右下角縮放節點　Esc 取消",
            Self::Pdf(_) => "在 PDF 頁面上拖曳選區　使用 PDF 操作換頁或檢視本頁標註",
            Self::Graph(_) => "捲動查看更多連結，點選節點即可開啟筆記",
        }
    }
}

unsafe fn choose<T>(
    owner: HWND,
    anchor: HWND,
    items: &[T],
    label: impl Fn(&T) -> String,
) -> Option<usize> {
    unsafe {
        let labels = items.iter().map(label).collect::<Vec<_>>();
        let items = labels.iter().enumerate()
            .map(|(i, title)| (i + 1, title.as_str())).collect::<Vec<_>>();
        let index = popup(owner, anchor, None, &items).checked_sub(1)?;
        (index < items.len()).then_some(index)
    }
}

impl App {
    unsafe fn activate_workspace(&mut self, view: WorkspaceView) {
        unsafe {
            self.cancel_workspace_drag();
            self.extras.workspace = Some(view);
            self.extras.board = true;
            self.present();
        }
    }

    unsafe fn open_kanban(&mut self, id: i64, title: String) -> Result<()> {
        let saved = self.active_collection_view(id);
        unsafe { self.open_collection_layout(id, title, saved) }
    }

    pub(super) unsafe fn collection_picker(&mut self) -> Result<()> {
        unsafe {
            let anchor = self.widgets.files;
            match popup(self.hwnd, anchor, None, &[(1, "開啟集合看板…"), (2, "新增集合…")]) {
                1 => {
                    let collections = self.notebook.collections()?;
                    if let Some(index) = choose(self.hwnd, anchor, &collections, |item| item.1.clone()) {
                        let (id, title) = &collections[index];
                        self.open_collection_layout(*id, title.clone(), None)?;
                    } else if collections.is_empty() {
                        label(self.widgets.status, "先新增一個集合。");
                    }
                }
                2 => {
                    if let Some(title) = crate::dialog::prompt(self.hwnd, "新增集合", "集合名稱", "", 120)? {
                        let id = self.notebook.create_collection(&title)?;
                        self.open_kanban(id, title.trim().to_owned())?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }

    pub(super) unsafe fn export_vault_zip_ui(&mut self) -> Result<()> {
        unsafe {
            let vaults = self.notebook.imported_vaults()?;
            if vaults.is_empty() {
                label(self.widgets.status, "目前沒有已匯入的 Obsidian 筆記庫。");
                return Ok(());
            }
            let Some(index) = choose(self.hwnd, self.widgets.files, &vaults, |(id, title)| {
                format!("{title}  (#{id})")
            }) else { return Ok(()); };
            let Some(path) = file_dialog(self.hwnd, "Markdown ZIP\0*.zip\0\0", "zip", Some("vault.zip"))?
            else { return Ok(()); };
            let count = self.notebook.export_vault_markdown(vaults[index].0, &path)?;
            label(self.widgets.status, &format!("已匯出 {count} 篇 Markdown，保留原筆記庫路徑。"));
            Ok(())
        }
    }

    pub(super) unsafe fn open_canvas(&mut self, path: Option<&Path>) -> Result<()> {
        unsafe {
            let (id, title, doc) = if let Some(path) = path {
                let doc = crate::canvas::read(path)?;
                let title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Canvas");
                let id = self.notebook.save_canvas(title, &doc)?;
                (id, crate::store::clean_title(title), doc)
            } else {
                let choices = self.notebook.saved_canvases()?;
                let Some(index) = choose(self.hwnd, self.widgets.files, &choices, |item| item.1.clone()) else {
                    if choices.is_empty() { label(self.widgets.status, "目前沒有匯入的 Canvas。"); }
                    return Ok(());
                };
                let (id, title) = &choices[index];
                (*id, title.clone(), self.notebook.read_canvas(*id)?)
            };
            self.activate_canvas(id, title, doc)
        }
    }

    unsafe fn activate_canvas(&mut self, id: i64, title: String, doc: crate::canvas::Document) -> Result<()> {
        let bundle = self.notebook.canvas_bundle_view(id)?;
        unsafe {
            self.activate_workspace(WorkspaceView::Canvas(Canvas {
                id, title, doc, bundle, projection: Cell::new(None), drag: None,
            }));
        }
        Ok(())
    }

    pub(super) unsafe fn import_canvas_bundle_ui(&mut self) -> Result<()> {
        unsafe {
            let Some(path) = file_dialog(self.hwnd, "JSON Canvas ZIP\0*.zip\0\0", "zip", None)? else {
                return Ok(());
            };
            let imported = self.notebook.import_canvas_bundle(&path)?;
            let doc = self.notebook.read_canvas(imported.id)?;
            self.activate_canvas(imported.id, imported.title, doc)?;
            crate::dialog::show_text(self.hwnd, "Canvas ZIP 匯入報告", &imported.report.summary())?;
            Ok(())
        }
    }

    pub(super) unsafe fn open_anchored_note(&mut self, id: i64, anchor: Option<&str>) -> Result<()> {
        unsafe { self.open_note_destination(id, anchor, crate::markdown::AnchorSyntax::Wiki, false) }
    }

    pub(super) unsafe fn open_anchored_link(
        &mut self, id: i64, anchor: Option<&str>, syntax: crate::markdown::AnchorSyntax,
    ) -> Result<()> {
        unsafe {
            if let Some(anchor) = anchor.filter(|anchor| anchor.starts_with("#^")) {
                self.preview_block_link(id, anchor)
            } else {
                self.open_note_destination(id, anchor, syntax, true)
            }
        }
    }

    unsafe fn open_note_destination(
        &mut self, id: i64, anchor: Option<&str>, syntax: crate::markdown::AnchorSyntax, select_source: bool,
    ) -> Result<()> {
        use crate::markdown::{self, HeadingLookup, block_anchors::{BlockLookup, resolve_block}};
        use crate::store::navigation::{editor_source_selection, outline};

        unsafe {
            let Some(note) = self.notebook.note(id)?.filter(|note| note.row.trash_root.is_none()) else {
                return Err("這篇來源筆記已不存在或在垃圾桶。".into());
            };
            let same_note = self.current.as_ref().is_some_and(|current| current.row.id == id);
            self.open_import(id, "已開啟來源筆記。");
            let Some(anchor) = anchor else { return Ok(()); };
            let Some(selected) = self.current.as_ref().filter(|note| note.row.id == id && note.row.trash_root.is_none()) else {
                return Err("來源筆記已變更，請重新開啟連結。".into());
            };
            // The same-note selection fast path can keep an older snapshot.
            let stale = selected.version < note.version;
            if stale {
                self.current = Some(note);
            }
            if same_note || stale {
                self.present();
            }
            let body = &self.current.as_ref().unwrap().body;
            let (section, selection) = if anchor.starts_with("#^") {
                match resolve_block(body, anchor) {
                    BlockLookup::Found { source, .. } => {
                        let selection = if select_source {
                            let cursor = editor_source_selection(body, source.clone())
                                .ok_or("編輯器的區塊位置已變更，請重新開啟連結。")?;
                            Some((cursor.start, cursor.end, format!("區塊連結：第 {} 行，{}", cursor.line, &anchor[1..])))
                        } else {
                            None
                        };
                        (source, selection)
                    }
                    outcome => {
                        let message = match outcome {
                            BlockLookup::Ambiguous => "這個區塊 ID 重複，無法唯一定位。",
                            BlockLookup::Missing => "找不到這個區塊 ID。",
                            _ => "這個區塊 ID 僅能使用英文字母、數字或連字號。",
                        };
                        label(self.widgets.status, &format!("已開啟來源筆記。{message}"));
                        return Ok(());
                    }
                }
            } else {
                match markdown::resolve_heading(body, anchor, syntax) {
                    HeadingLookup::Found { ordinal, section } => {
                        let selection = if select_source {
                            let heading = outline(body).into_iter().nth(ordinal)
                                .ok_or("編輯器的標題位置已變更，請重新開啟連結。")?;
                            Some((heading.start, heading.end, format!("標題連結：第 {} 行，{}", heading.line, heading.title)))
                        } else {
                            None
                        };
                        (section, selection)
                    }
                    outcome => {
                        let message = match outcome {
                            HeadingLookup::Ambiguous => "這個標題連結有多個符合項目。",
                            HeadingLookup::Missing => "找不到這個標題。",
                            _ => "這個片段無法解析為標題。",
                        };
                        label(self.widgets.status, &format!("已開啟來源筆記。{message}"));
                        return Ok(());
                    }
                }
            };
            let excerpt = &body[section];
            let end = excerpt.char_indices().nth(8192).map_or(excerpt.len(), |(i, _)| i);
            let excerpt = excerpt[..end].replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n");
            if let Some((start, end, status)) = selection && self.focus_source_selection(start, end)? {
                label(self.widgets.status, &status);
            }
            crate::dialog::show_text(self.hwnd, anchor, &excerpt)
        }
    }

    pub(super) unsafe fn workspace_menu(&mut self) -> Result<()> {
        unsafe {
            let hwnd = self.hwnd;
            let anchor = self.widgets.links;
            match self.extras.workspace.as_ref() {
                Some(WorkspaceView::Canvas(view)) => {
                    let id = view.id;
                    let initial_binding = self.notebook.canvas_vault(id)?;
                    let mut choices = vec![
                        (1, "匯出原始 JSON Canvas…"), (2, "檢視格式摘要"), (3, "移除已匯入的 Canvas…"),
                        (7, "編輯 Canvas…"), (8, "將畫布調整至視窗"),
                    ];
                    if view.bundle.is_some() {
                        choices.extend([
                            (9, "匯出 JSON Canvas ZIP…"), (10, "檢視附件與相容性"), (11, "儲存封裝附件…"),
                        ]);
                    }
                    choices.splice(3..3, [
                        (4, "綁定匯入的 Vault…"), (5, "開啟畫布來源筆記…"), (6, "解除 Vault 綁定"),
                    ]);
                    let action = popup(hwnd, anchor, None, &choices);
                    if action == 1 && let Some(path) =
                        file_dialog(hwnd, "JSON Canvas\0*.canvas\0\0", "canvas", Some("canvas.canvas"))?
                    {
                        crate::canvas::export(&path, &self.notebook.read_canvas(id)?)?;
                        label(self.widgets.status, "已匯出原始 JSON Canvas。");
                    } else if action == 2 {
                        let body = format!("節點 {}\r\n連線 {}\r\n\r\n預覽最多繪製 500 個節點及 1,000 條連線。匯出保留全部原始內容。",
                            view.doc.nodes, view.doc.edges);
                        crate::dialog::show_text(hwnd, "Canvas 格式摘要", &body)?;
                    } else if action == 3 && message(
                        hwnd, "移除已匯入的 Canvas", "將這份匯入的副本移出工作區？",
                        MB_OKCANCEL | MB_ICONQUESTION,
                    ) == IDOK {
                        self.notebook.remove_canvas(id, &view.doc)?;
                        self.extras.workspace = None;
                        self.extras.board = false;
                        self.present();
                    } else if action == 4 {
                        let vaults = self.notebook.imported_vaults()?;
                        if let Some(index) = choose(hwnd, anchor, &vaults, |(id, title)| format!("{title}  ({id})")) {
                            self.notebook.compare_set_canvas_vault(id, initial_binding, Some(vaults[index].0))?;
                            label(self.widgets.status, "已將畫布連到這個匯入的 Vault。");
                        } else if vaults.is_empty() {
                            let hint = if view.bundle.is_some() {
                                "ZIP 內的附件不會自動建立筆記。請先匯入 Markdown Vault，再綁定來源。"
                            } else { "先匯入含 Markdown 筆記的 Vault。" };
                            label(self.widgets.status, hint);
                        }
                    } else if action == 5 {
                        let (expected, links) = self.notebook.canvas_file_link_snapshot(id)?;
                        if let Some(index) = choose(hwnd, anchor, &links, |link| {
                            let suffix = link.subpath.as_deref().unwrap_or("");
                            let label = link.label.as_deref().unwrap_or(&link.file);
                            format!("{label} 〔{}〕 {suffix}", link.node_id)
                        }) {
                            let link = self.notebook.resolve_canvas_file_link(id, expected, &links[index].node_id)?;
                            let target = link.note_id.ok_or("請先綁定含有這個精確路徑的 Vault。")?;
                            self.open_anchored_note(target, link.subpath.as_deref())?;
                        } else if links.is_empty() {
                            label(self.widgets.status, "畫布沒有檔案節點。");
                        }
                    } else if action == 6 {
                        self.notebook.compare_set_canvas_vault(id, initial_binding, None)?;
                        label(self.widgets.status, "已解除畫布的 Vault 綁定。");
                    } else if action == 7 {
                        self.cancel_canvas_drag();
                        self.canvas_edit_menu(id)?;
                    } else if action == 8 {
                        view.projection.set(None);
                        self.present();
                    } else if action == 9 {
                        if let Some(path) = file_dialog(hwnd, "JSON Canvas ZIP\0*.zip\0\0", "zip", Some("canvas.zip"))? {
                            self.notebook.export_canvas_bundle(id, &path)?;
                            label(self.widgets.status, "已匯出 JSON Canvas ZIP 及保存的附件。");
                        }
                    } else if action == 10 {
                        let report = self.notebook.canvas_bundle_report(id)?.ok_or("這份 Canvas 沒有封裝附件。")?;
                        crate::dialog::show_text(hwnd, "Canvas ZIP 附件與相容性", &report.summary())?;
                    } else if action == 11 {
                        let bundle = view.bundle.as_ref().ok_or("這份 Canvas 沒有封裝附件。")?;
                        if bundle.asset_paths.is_empty() {
                            label(self.widgets.status, "這份 ZIP 沒有可另存的附件。");
                        } else if let Some(index) = super::navigation_ui::choose_navigation_page(
                            hwnd, anchor, bundle.asset_paths.len(), |i| &bundle.asset_paths[i],
                        ) {
                            let member = &bundle.asset_paths[index];
                            let name = member.rsplit('/').next().ok_or("封裝附件缺少檔名。")?;
                            if let Some(path) = file_dialog(hwnd, "所有檔案\0*.*\0\0", "", Some(name))? {
                                self.notebook.save_canvas_bundle_asset(id, member, &path)?;
                                label(self.widgets.status, "已另存封裝內的原始附件。");
                            }
                        }
                    }
                }
                Some(WorkspaceView::Pdf(_)) => self.pdf_menu()?,
                Some(WorkspaceView::Graph(_)) => self.graph_menu()?,
                Some(WorkspaceView::Kanban(view)) => {
                    let (id, title) = (view.id, view.title.clone());
                    self.kanban_menu(id, title)?;
                }
                _ => {}
            }
            Ok(())
        }
    }

    unsafe fn kanban_menu(&mut self, id: i64, title: String) -> Result<()> {
        unsafe {
            let hwnd = self.hwnd;
            let anchor = self.widgets.links;
            let week = matches!(&self.extras.workspace,Some(WorkspaceView::Kanban(view)) if view.week.is_some());
            let timeline = matches!(&self.extras.workspace,Some(WorkspaceView::Kanban(view)) if view.timeline.is_some());
            let previous = if timeline { "時間軸上個月" } else if week { "月曆上一週" } else { "月曆上個月" };
            let next = if timeline { "時間軸下個月" } else if week { "月曆下一週" } else { "月曆下個月" };
            let open_date = if timeline { "開啟時間軸資料列…" } else { "開啟月曆日期筆記…" };
            let edit_dates = if timeline { "編輯時間軸資料列日期…" } else { "編輯月曆資料列日期…" };
            let action = popup(hwnd, anchor, None, &[
                (1, "加入筆記…"), (2, "移動到其他狀態…"), (3, "開啟資料列…"),
                (4, "新增公式…"), (5, "檢視公式結果…"), (6, "從集合移除…"),
                (7, "上一欄"), (8, "下一欄"), (9, "重新整理"), (10, "移除集合公式…"),
                (11, "集合檢視…"), (12, "編輯集合公式…"), (13, "切換資料列核取方塊…"),
                (14, previous), (15, next), (16, open_date), (17, edit_dates),
                (18, "編輯資料列的型別欄位…"), (19, "編輯資料列關聯…"), (20, "跳至時間軸月份…"),
                (21, "資料列範本…"),
            ]);
            match action {
                1 => {
                    let notes = self.notebook.rows(View::All, "")?;
                    if let Some(index) = choose(hwnd, anchor, &notes, |note| format!("{}  ({})", note.title, note.id))
                        && let Some(status) = crate::dialog::prompt(hwnd, "加入集合", "狀態欄名稱", "待處理", 256)?
                    {
                        self.notebook.add_to_collection(id, notes[index].id, &status)?;
                    }
                }
                2 | 3 | 6 => {
                    let rows = self.visible_collection_rows(id)?;
                    let Some(index) = choose(hwnd, anchor, &rows, |(row, status)| {
                        format!("{status}　{}  ({})", row.title, row.id)
                    }) else { return Ok(()); };
                    let (row, old_status) = &rows[index];
                    match action {
                        2 => {
                            if let Some(status) = crate::dialog::prompt(
                                hwnd, "移動資料列", "新狀態欄，可輸入現有或新名稱", old_status, 256,
                            )? {
                                self.notebook.move_collection_note(id, row.id, old_status, &status)?;
                            }
                        }
                        3 => {
                            let note = row.id;
                            self.open_import(note, "已開啟集合資料列");
                            return Ok(());
                        }
                        _ => self.notebook.remove_from_collection(id, row.id, old_status)?,
                    }
                }
                4 => {
                    let Some(name) = crate::dialog::prompt(hwnd, "新增公式", "公式名稱", "", MAX_KEY_BYTES)? else {
                        return Ok(());
                    };
                    if let Some(expr) = crate::dialog::prompt(
                        hwnd, "新增公式", "範例：typed(\"工時\") * 2 或 relation_sum(\"子項\",\"工時\")", "", 4096,
                    )? {
                        self.notebook.add_collection_formula(id, &name, &expr)?;
                    }
                }
                5 => return self.show_formula(id),
                7 | 8 => {
                    if let Some(WorkspaceView::Kanban(view)) = &mut self.extras.workspace {
                        view.column = if action == 7 { view.column.saturating_sub(1) } else {
                            (view.column + 1).min(view.lanes.len().saturating_sub(1))
                        };
                    }
                }
                9 => {}
                10 => {
                    let definitions = self.notebook.collection_formulas(id)?;
                    if let Some(index) = choose(hwnd, anchor, &definitions, |(name, _)| name.clone()) {
                        let (name, expr) = &definitions[index];
                        self.notebook.remove_collection_formula(id, name, expr)?;
                    }
                }
                11 => return self.collection_view_menu(id, title),
                13 => return self.collection_checkbox_menu(id, title),
                14 => return self.shift_collection_month(id,title,-1),
                15 => return self.shift_collection_month(id,title,1),
                16 => return self.open_collection_date(None),
                17 => return self.edit_collection_row_dates(id, title),
                18 => return self.edit_collection_row_field(id, title),
                19 => return self.edit_collection_row_relations(id, title),
                20 => return self.jump_collection_timeline(id, title),
                21 => return self.collection_template_menu(id),
                12 => {
                    let definitions = self.notebook.collection_formulas(id)?;
                    let Some(index) = choose(hwnd, anchor, &definitions, |(name, _)| name.clone()) else {
                        return Ok(());
                    };
                    let (name, expression) = &definitions[index];
                    let Some(next) = crate::dialog::prompt(hwnd, "編輯集合公式", name, expression, 4096)?
                    else { return Ok(()); };
                    if let Err(error) = self.notebook.edit_collection_formula(id, name, expression, &next) {
                        crate::dialog::show_text(hwnd, "未儲存的公式（可複製）", &next)?;
                        return Err(error);
                    }
                    self.show_formula(id)?;
                    return Ok(());
                }
                _ => return Ok(()),
            }
            self.open_kanban(id, title)
        }
    }

    unsafe fn show_formula(&self, collection: i64) -> Result<()> {
        unsafe {
            let formulas = self.notebook.collection_formulas(collection)?;
            let Some(index) = choose(self.hwnd, self.widgets.links, &formulas, |(name, _)| name.clone()) else {
                return Ok(());
            };
            let (name, expr) = &formulas[index];
            let mut result = format!("{name}\r\n{expr}\r\n\r\n");
            for (note, _) in self.visible_collection_rows(collection)? {
                let value = self.notebook.evaluate_note_formula(note.id, expr)
                    .unwrap_or_else(|error| format!("錯誤：{error}"));
                writeln!(result, "{}\t{value}\r", note.title)?;
            }
            crate::dialog::show_text(self.hwnd, "集合公式結果", &result)
        }
    }

    pub(super) unsafe fn relation_menu(&mut self, source: i64) -> Result<()> {
        unsafe {
            let hwnd = self.hwnd;
            let anchor = self.widgets.more;
            let action = popup(hwnd, anchor, None, &[
                (1, "新增關聯…"), (2, "開啟關聯筆記…"), (3, "移除關聯…"),
                (4, "重新計算關聯彙總…"),
            ]);
            if action == 1 {
                let notes = self.notebook.rows(View::All, "")?.into_iter()
                    .filter(|row| row.id != source).collect::<Vec<_>>();
                if let Some(index) = choose(hwnd, anchor, &notes, |row| format!("{}  ({})", row.title, row.id))
                    && let Some(name) = crate::dialog::prompt(hwnd, "新增關聯", "關聯名稱", "", 256)?
                {
                    self.notebook.set_relation(source, &name, notes[index].id, true)?;
                }
            } else if matches!(action, 2 | 3) {
                let refs = self.notebook.relations(source)?;
                if let Some(index) = choose(hwnd, anchor, &refs, |(name, row)| format!("{name}　{}  ({})", row.title, row.id)) {
                    let (name, target) = &refs[index];
                    if action == 2 {
                        self.open_import(target.id, "已開啟關聯筆記");
                    } else {
                        self.notebook.set_relation(source, name, target.id, false)?;
                    }
                }
            } else if action == 4 {
                let Some(name) = crate::dialog::prompt(hwnd, "關聯彙總", "關聯名稱", "", 256)? else {
                    return Ok(());
                };
                let Some(property) = crate::dialog::prompt(hwnd, "關聯彙總", "數值屬性名稱（空白只顯示筆數）", "", 256)? else {
                    return Ok(());
                };
                let total = if property.trim().is_empty() {
                    self.notebook.rollup_count(source, &name)?.to_string()
                } else {
                    self.notebook.rollup_sum(source, &name, &property)?.to_string()
                };
                let title = if property.trim().is_empty() { "有效關聯筆數" } else { "關聯屬性總和" };
                crate::dialog::show_text(hwnd, "關聯彙總", &format!("{name}\r\n{title}　{total}"))?;
            }
            Ok(())
        }
    }

    pub(super) unsafe fn attachments_menu(&mut self, note: i64) -> Result<()> {
        unsafe {
            let hwnd = self.hwnd;
            let anchor = self.widgets.more;
            let action = popup(hwnd, anchor, None, &[(1, "加入本機附件…"), (2, "管理附件…")]);
            if action == 1 {
                return self.add_note_attachment(note);
            }
            if action != 2 { return Ok(()); }
            let files = self.notebook.list_attachments(note)?;
            if let Some(index) = choose(hwnd, anchor, &files, |file| format!("{}  ({} B)", file.name, file.size)) {
                let file = &files[index];
                let mut options = vec![(1, "儲存附件副本…"), (2, "刪除附件…")];
                if file.kind == "pdf" {
                    options.extend([
                        (3, "新增 PDF 頁碼摘錄…"), (4, "檢視 PDF 摘錄"),
                        (5, "刪除 PDF 摘錄…"), (6, "新增 PDF 矩形座標摘錄…"),
                        (7, "開啟 PDF 頁面並標註"),
                        (8, "開啟已保存的 PDF 標註…"),
                    ]);
                }
                let origin = self.notebook.attachment_origin(file.id)?;
                if origin.is_some() {
                    options.push((9, "查看匯入來源路徑"));
                }
                let action = popup(hwnd, anchor, None, &options);
                match action {
                    1 => {
                        if let Some(path) = file_dialog(hwnd, "所有檔案\0*.*\0\0", "", Some(&file.name))? {
                            self.notebook.save_attachment_copy(file.id, &path)?;
                            label(self.widgets.status, "附件副本已儲存。");
                        }
                    }
                    2 => {
                        if message(hwnd, "刪除附件", "附件及其 PDF 摘錄將從工作區移除。確定刪除？",
                                   MB_OKCANCEL | MB_ICONQUESTION) == IDOK {
                            self.notebook.remove_attachment(file.id)?;
                        }
                    }
                    3 | 6 => {
                        let Some(page) = crate::dialog::prompt(hwnd, "PDF 摘錄", "來源 PDF 頁碼（從 1 起算）", "", 7)? else {
                            return Ok(());
                        };
                        let page: u32 = page.trim().parse().map_err(|_| "頁碼必須是正整數。")?;
                        let Some(quote) = crate::dialog::prompt_multiline(hwnd, "PDF 摘錄", "貼上原文，或留下空白", "", 4096)? else {
                            return Ok(());
                        };
                        if let Some(comment) = crate::dialog::prompt_multiline(hwnd, "PDF 摘錄", "輸入批註", "", 4096)? {
                            if action == 3 {
                                self.notebook.add_pdf_annotation(file.id, page, &quote, &comment)?;
                            } else {
                                let Some(spec) = crate::dialog::prompt(
                                    hwnd, "PDF 矩形座標摘錄", "輸入 x,y,寬,高；左上為原點，用 0 到 1 的比例", "", 150,
                                )? else { return Ok(()); };
                                let values = spec.split(',').map(|value| value.trim().parse::<f64>())
                                    .collect::<std::result::Result<Vec<_>, _>>()?;
                                let &[x, y, width, height] = values.as_slice() else {
                                    return Err("請輸入四個以逗號分隔的數字。".into());
                                };
                                self.notebook.add_pdf_annotation_with_region(
                                    file.id, page, &quote, &comment, crate::store::PdfRegion { x, y, width, height },
                                )?;
                            }
                        }
                    }
                    4 => {
                        let notes = self.notebook.list_pdf_annotations(file.id)?;
                        let mut text = String::new();
                        for entry in &notes {
                            write!(text, "第 {} 頁\r\n{}\r\n{}\r\n", entry.page, entry.quote, entry.comment)?;
                            if let Some(region) = self.notebook.pdf_region(entry.id)? {
                                write!(text, "頁面比例：{},{},{},{}\r\n",
                                    region.x, region.y, region.width, region.height)?;
                            }
                            text.push_str("\r\n");
                        }
                        if notes.is_empty() { text.push_str("這個 PDF 尚無摘錄。"); }
                        crate::dialog::show_text(hwnd, "PDF 頁碼摘錄", &text)?;
                    }
                    5 => {
                        let notes = self.notebook.list_pdf_annotations(file.id)?;
                        if let Some(index) = choose(hwnd, anchor, &notes, |entry| {
                            let text = entry.quote.lines().next().unwrap_or("").chars().take(48).collect::<String>();
                            format!("第 {} 頁　{text}", entry.page)
                        }) && message(hwnd, "刪除摘錄", "將這筆摘錄從工作區移除？",
                                      MB_OKCANCEL | MB_ICONQUESTION) == IDOK {
                            self.notebook.remove_pdf_annotation(notes[index].id)?;
                        }
                    }
                    7 => self.open_pdf_view(file.id, 1)?,
                    8 => self.jump_pdf_annotation(file.id, anchor)?,
                    9 => {
                        if let Some(path) = origin {
                            crate::dialog::show_text(hwnd, "匯入時的相對附件路徑", &path)?;
                        }
                    }
                    _ => {}
                }
            } else if files.is_empty() {
                label(self.widgets.status, "這篇筆記尚無附件。");
            }
            Ok(())
        }
    }

    pub(super) unsafe fn add_note_attachment(&mut self, id: i64) -> Result<()> {
        unsafe {
            if let Some(path) = file_dialog(
                self.hwnd, "附件\0*.pdf;*.png;*.jpg;*.jpeg;*.txt;*.md;*.markdown\0\0", "", None,
            )? {
                self.notebook.import_attachment(id, &path)?;
                label(self.widgets.status, "附件已保存到本機工作區。");
            }
            Ok(())
        }
    }

    pub(super) unsafe fn workspace_timer(&mut self) {
        unsafe { self.poll_pdf(); }
    }

    pub(super) unsafe fn cancel_workspace_drag(&mut self) -> bool {
        unsafe { self.cancel_timeline_drag() || self.cancel_canvas_drag() }
    }

    pub(super) unsafe fn workspace_input(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) {
        if matches!(self.extras.workspace, Some(WorkspaceView::Pdf(_))) {
            if let Err(error) = unsafe { self.pdf_input(msg, lp) } {
                unsafe { self.show_error(error); }
            }
            return;
        }
        if matches!(self.extras.workspace, Some(WorkspaceView::Canvas(_))) {
            if let Err(error) = unsafe { self.canvas_input(msg, lp) } {
                unsafe { self.show_error(error); }
            }
            return;
        }
        if matches!(self.extras.workspace, Some(WorkspaceView::Graph(_))) {
            if let Err(error) = unsafe { self.graph_input(msg, wp, lp) } {
                unsafe { self.show_error(error); }
            }
            return;
        }
        if matches!(&self.extras.workspace, Some(WorkspaceView::Kanban(view)) if view.timeline.is_some()) {
            match unsafe { self.timeline_input(msg, lp) } {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => {
                    unsafe { self.show_error(error); }
                    return;
                }
            }
        }
        if msg == WM_LBUTTONUP && matches!(self.extras.workspace, Some(WorkspaceView::Kanban(_))) {
            if let Err(error) = unsafe { self.click_collection_view(lp) } {
                unsafe { self.show_error(error); }
            }
            return;
        }
        if msg == WM_MOUSEWHEEL && let Some(WorkspaceView::Kanban(view)) = &mut self.extras.workspace {
            let delta = (wp >> 16) as u16 as i16;
            let max = view.max_scroll_row();
            view.row = if delta > 0 { view.row.saturating_sub(1) } else { (view.row + 1).min(max) };
            unsafe { InvalidateRect(self.hwnd, null(), 1); }
        }
    }

    pub(super) unsafe fn paint_workspace(&self, dc: HDC, area: RECT, view: &WorkspaceView) {
        unsafe {
            fill(dc, &area, color(246, 248, 243));
            if let WorkspaceView::Pdf(view) = view {
                self.paint_pdf(dc, area, view);
                return;
            }
            if let WorkspaceView::Graph(view) = view {
                self.paint_graph(dc, area, view);
                return;
            }
            if let WorkspaceView::Kanban(view) = view {
                if let Some((month, rows)) = &view.timeline {
                    self.paint_collection_timeline(dc, area, view, *month, rows);
                    return;
                }
                if let Some((week, days)) = &view.week {
                    self.paint_collection_week(dc,area,*week,days);
                    return;
                }
                if let Some((month, days)) = &view.calendar {
                    self.paint_collection_calendar(dc,area,*month,days);
                    return;
                }
                if view.is_list() {
                    self.paint_collection_list(dc,area,view);
                    return;
                }
                if view.is_table() {
                    self.paint_collection_table(dc, area, view);
                    return;
                }
                let width = (area.right - area.left) / 3;
                if width <= 0 { return; }
                if view.lanes.is_empty() {
                    let mut hint = area;
                    hint.left += self.scaled(16); hint.top += self.scaled(16);
                    let message = if view.saved_view.is_some() {
                        "這個檢視沒有符合的筆記。"
                    } else {
                        "從「看板操作」加入第一筆筆記。"
                    };
                    draw_text(dc, self.fonts[4], SOFT, message, &mut hint, DT_SINGLELINE);
                }
                for (column, (status, rows)) in view.lanes.iter().skip(view.column).take(3).enumerate() {
                    let left = area.left + column as i32 * width + self.scaled(7);
                    let mut title = RECT { left, top: area.top + self.scaled(10), right: left + width - self.scaled(16),
                                           bottom: area.top + self.scaled(42) };
                    fill(dc, &title, MINT);
                    draw_text(dc, self.fonts[4], TEAL, &format!("{status}  ·  {}", rows.len()), &mut title,
                              DT_SINGLELINE | DT_END_ELLIPSIS);
                    let step = self.scaled(45).max(1);
                    for (offset, row) in rows.iter().skip(view.row).take(((area.bottom - title.bottom) / step).max(0) as usize).enumerate() {
                        let top = title.bottom + offset as i32 * step + self.scaled(6);
                        let mut card = RECT { left, top, right: title.right, bottom: top + self.scaled(35) };
                        fill(dc, &card, PAPER);
                        card.left += self.scaled(7);
                        draw_text(dc, self.fonts[4], INK, &row.title, &mut card, DT_SINGLELINE | DT_END_ELLIPSIS);
                    }
                }
                return;
            }
            let WorkspaceView::Canvas(view) = view else { return; };
            self.paint_canvas(dc, area, view);
        }
    }
}
