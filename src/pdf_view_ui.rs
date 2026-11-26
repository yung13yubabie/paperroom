use super::*;
use crate::pdf_geometry::PageBox;
use crate::pdf_render::{RenderedPage, TextSelection};
use crate::store::{PdfPageText, PdfRegion};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};

pub(crate) const PDF_TIMER: usize = 3;
static RENDERING: AtomicBool = AtomicBool::new(false);
type RenderResult = std::result::Result<RenderedPage, String>;

pub(crate) struct PdfView {
    pub title: String,
    attachment: i64,
    source: Arc<Vec<u8>>,
    annotation: Option<i64>,
    paired_excerpt: Option<i64>,
    adjacent: Option<AdjacentDraft>,
    name: String,
    page: u32,
    raster: Option<RenderedPage>,
    pending: Option<Receiver<RenderResult>>,
    error: Option<String>,
    regions: Vec<PdfRegion>,
    text_marks: Vec<PdfRegion>,
    marked_bgra: Option<Vec<u8>>,
    focus: Vec<PdfRegion>,
    removed_annotation_preview: bool,
    painted: Cell<Option<PageBox>>,
    drag: Option<Drag>,
    text_mode: bool,
    text_selection: Option<TextSelection>,
}

#[derive(Clone)]
struct AdjacentDraft {
    source: Arc<Vec<u8>>,
    page: u32,
    selection: TextSelection,
}

#[derive(Clone, Copy)]
struct Drag {
    page_box: PageBox,
    start: [i32; 2],
    end: [i32; 2],
}

struct Rendering;
impl Drop for Rendering {
    fn drop(&mut self) {
        RENDERING.store(false, Ordering::Release);
    }
}

impl Drag {
    fn select_text(self, frame: &RenderedPage) -> Option<TextSelection> {
        let normalize = |point: [i32; 2]| [
            f64::from(point[0] - self.page_box.x) / f64::from(self.page_box.width),
            f64::from(point[1] - self.page_box.y) / f64::from(self.page_box.height),
        ];
        frame.text.select(normalize(self.start), normalize(self.end))
    }
}

impl PdfView {
    fn page_box(&self, area: RECT, padding: i32) -> Option<PageBox> {
        let frame = self.raster.as_ref()?;
        PageBox::fit(
            [area.left.saturating_add(padding), area.top.saturating_add(padding),
             area.right.saturating_sub(padding), area.bottom.saturating_sub(padding)],
            [frame.width, frame.height],
        )
    }
}

fn preview_label(removed_annotation: bool) -> &'static str {
    if removed_annotation { "已移除標註預覽 · " } else { "" }
}

impl App {
    pub(super) unsafe fn open_pdf_view(&mut self, attachment: i64, page: u32) -> Result<()> {
        unsafe { self.open_pdf_view_at(attachment, page, Vec::new(), false, None, None) }
    }

    pub(in crate::window) unsafe fn open_pdf_annotation_at(
        &mut self, attachment: i64, page: u32, annotation: i64,
    ) -> Result<()> {
        let focus = self.notebook.pdf_annotation_focus_regions(attachment, page, annotation)?;
        unsafe { self.open_pdf_view_at(attachment, page, focus, false, Some(annotation), None) }
    }

    unsafe fn open_pdf_view_at(
        &mut self, attachment: i64, page: u32, focus: Vec<PdfRegion>, removed_annotation_preview: bool,
        annotation: Option<i64>, adjacent: Option<AdjacentDraft>,
    ) -> Result<()> {
        unsafe {
            let (name, bytes) = self.notebook.read_pdf_attachment(attachment)?;
            let source = if let Some(draft) = &adjacent {
                if bytes.as_slice() != draft.source.as_slice() {
                    return Err("PDF 原始位元已變更。請重新開啟原檔再選取。".into());
                }
                Arc::clone(&draft.source)
            } else {
                Arc::new(bytes)
            };
            let render_source = Arc::clone(&source);
            let regions = self.notebook.pdf_area_regions_on_page(attachment, page)?;
            let text_marks = self.notebook.pdf_text_marks_on_page(attachment, page)?;
            if RENDERING.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
                return Err("另一個 PDF 頁面正在讀取。".into());
            }
            let guard = Rendering;
            let (tx, rx) = sync_channel(1);
            std::thread::Builder::new().name("paperroom-pdf".to_owned()).spawn(move || {
                let result = crate::pdf_render::render_page(render_source, page).map_err(|error| error.to_string());
                drop(guard);
                let _ = tx.send(result);
            })?;
            if SetTimer(self.hwnd, PDF_TIMER, 40, None) == 0 {
                return Err("無法安排 PDF 頁面顯示。".into());
            }
            let title = format!("{}{name} · 第 {page} 頁", preview_label(removed_annotation_preview));
            let text_mode = adjacent.is_some();
            self.activate_workspace(WorkspaceView::Pdf(PdfView {
                title, attachment, source, annotation, paired_excerpt: None, adjacent, name, page, raster: None, pending: Some(rx),
                error: None, regions, focus, removed_annotation_preview, painted: Cell::new(None), drag: None,
                text_mode, text_selection: None, text_marks, marked_bgra: None,
            }));
            Ok(())
        }
    }

    pub(super) unsafe fn poll_pdf(&mut self) {
        unsafe {
            let Some(WorkspaceView::Pdf(view)) = &mut self.extras.workspace else {
                KillTimer(self.hwnd, PDF_TIMER);
                return;
            };
            let Some(pending) = &view.pending else {
                KillTimer(self.hwnd, PDF_TIMER);
                return;
            };
            let result = match pending.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err("PDF 頁面未能完成讀取。".into()),
            };
            KillTimer(self.hwnd, PDF_TIMER);
            view.pending = None;
            match result {
                Ok(raster) => {
                    view.title = format!("{}{} · {}/{}", preview_label(view.removed_annotation_preview),
                        view.name, view.page, raster.page_count);
                    view.marked_bgra = raster.highlighted_bgra(&view.text_marks);
                    view.raster = Some(raster);
                }
                Err(error) => view.error = Some(error),
            }
            let selecting_next = view.adjacent.is_some() && view.raster.is_some();
            self.present();
            if selecting_next {
                label(self.widgets.status, "已保留上一頁。請選取本頁文字，再從 PDF 操作複製或保存兩頁。");
            }
        }
    }

    pub(super) unsafe fn jump_pdf_annotation(&mut self, attachment: i64, anchor: HWND) -> Result<()> {
        unsafe {
            let annotations = self.notebook.list_pdf_annotations(attachment)?;
            if annotations.is_empty() {
                label(self.widgets.status, "這個 PDF 尚無摘錄。");
                return Ok(());
            }
            if let Some(index) = choose(self.hwnd, anchor, &annotations, |entry| {
                let summary = if entry.quote.trim().is_empty() { &entry.comment } else { &entry.quote };
                let summary = summary.lines().next().unwrap_or("").chars().take(48).collect::<String>();
                format!("第 {} 頁　{summary}", entry.page)
            }) {
                let entry = &annotations[index];
                self.open_pdf_annotation_at(attachment, entry.page, entry.id)?;
            }
            Ok(())
        }
    }

    pub(in crate::window) unsafe fn search_pdf_excerpt_cards_ui(&mut self) -> Result<()> {
        unsafe {
            if let Some(note) = self.choose_pdf_excerpt_card(self.widgets.files)? {
                self.open_anchored_note(note, None)?;
            }
            Ok(())
        }
    }

    pub(in crate::window) unsafe fn add_existing_pdf_excerpt_ui(&mut self) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let Some(note) = self.choose_pdf_excerpt_card(self.widgets.links)? else { return Ok(()) };
            let created = self.notebook.place_existing_pdf_excerpt(board, note)?;
            self.refresh_board()?;
            let status = if created { "已將原摘錄卡放入目前白板。" } else { "這張摘錄卡已在目前白板。" };
            label(self.widgets.status, status);
            Ok(())
        }
    }

    unsafe fn choose_pdf_excerpt_card(&self, anchor: HWND) -> Result<Option<i64>> {
        unsafe {
            let Some(query) = crate::dialog::prompt(
                self.hwnd, "搜尋 PDF 摘錄卡", "輸入標題、原引文或原註解，留空顯示最近 64 張", "", 180,
            )? else { return Ok(None); };
            let cards = self.notebook.search_pdf_excerpt_cards(&query)?;
            if cards.is_empty() {
                label(self.widgets.status, "沒有符合的 PDF 摘錄卡。");
                return Ok(None);
            }
            Ok(choose(self.hwnd, anchor, &cards, |(id, title)| {
                format!("{title}（#{id}）")
            }).map(|index| cards[index].0))
        }
    }

    pub(in crate::window) unsafe fn restore_pdf_excerpt_ui(&mut self, note: i64) -> Result<()> {
        unsafe {
            if self.notebook.is_paired_pdf_excerpt(note)? {
                label(self.widgets.status, "跨頁摘錄可預覽保存的兩頁選區。此處不復原成對標註。");
                return Ok(());
            }
            let Some(source) = self.notebook.pdf_excerpt_recovery_preview(note)? else {
                label(self.widgets.status, "沒有可復原的標註。原標註可能仍在，或原附件、來源筆記或復原資料目前不可用。");
                return Ok(());
            };
            let details = format!(
                "{}\r\n第 {} 頁\r\n\r\n{}\r\n\r\n{}",
                source.name, source.page, source.quote, source.comment,
            );
            crate::dialog::show_text(self.hwnd, "預覽原 PDF 標註", &details)?;
            if message(
                self.hwnd, "復原 PDF 標註",
                "把這則標註復原到原 PDF？指向同一標註的摘錄卡也會重新連上。",
                MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
            ) == IDYES {
                self.notebook.restore_pdf_excerpt_annotation(note, source.origin)?;
                self.open_pdf_excerpt_source(note)?;
            }
            Ok(())
        }
    }

    pub(in crate::window) unsafe fn open_pdf_excerpt_source(&mut self, note: i64) -> Result<()> {
        unsafe {
            if self.open_paired_pdf_excerpt_page(note, None)? {
                return Ok(());
            }
            if let Some(source) = self.notebook.pdf_excerpt_source(note)? {
                self.open_anchored_note(source.note_id, None)?;
                if self.current.as_ref().is_some_and(|note| note.row.id == source.note_id) {
                    self.open_pdf_annotation_at(source.attachment_id, source.page, source.id)?;
                }
            } else if let Some(snapshot) = self.notebook.pdf_excerpt_snapshot(note)? {
                self.open_anchored_note(snapshot.note, None)?;
                if self.current.as_ref().is_some_and(|note| note.row.id == snapshot.note) {
                    self.open_pdf_view_at(snapshot.attachment, snapshot.page, snapshot.focus, true, None, None)?;
                }
            } else {
                label(self.widgets.status, "目前無法開啟 PDF 來源。這張摘錄卡仍可編輯。");
            }
            Ok(())
        }
    }

    /// Choose either fixed source page, or switch away from the specified current page.
    unsafe fn open_paired_pdf_excerpt_page(&mut self, card: i64, current: Option<u32>) -> Result<bool> {
        unsafe {
            let Some(pages) = self.notebook.paired_pdf_excerpt_pages(card)? else { return Ok(false) };
            if pages.is_empty() {
                label(self.widgets.status, "跨頁摘錄的原 PDF 目前無法開啟。摘錄卡仍可編輯。");
                return Ok(true);
            }
            let selection = if let Some(current) = current {
                if !pages.iter().any(|page| page.page == current) {
                    return Err("跨頁摘錄的來源頁面已改變，請從卡片重新開啟。".into());
                }
                pages.into_iter().find(|page| page.page != current)
            } else {
                let anchor = if self.extras.board { self.widgets.links } else { self.widgets.more };
                choose(self.hwnd, anchor, &pages, |page| format!("第 {} 頁", page.page))
                    .and_then(|index| pages.into_iter().nth(index))
            };
            if let Some(page) = selection {
                self.open_anchored_note(page.source_note, None)?;
                if self.current.as_ref().is_some_and(|note| note.row.id == page.source_note) {
                    self.open_pdf_view_at(page.attachment, page.page, page.focus,
                        page.annotation.is_none(), page.annotation, None)?;
                    if let Some(WorkspaceView::Pdf(view)) = &mut self.extras.workspace {
                        view.paired_excerpt = Some(card);
                    }
                }
            }
            Ok(true)
        }
    }

    unsafe fn choose_pdf_excerpt_board(&self) -> Result<Option<(i64, String)>> {
        unsafe {
            let boards = self.notebook.list_boards()?;
            let result = choose(self.hwnd, self.widgets.links, &boards, |board| board.title.clone());
            Ok(result.map(|index| (boards[index].id, boards[index].title.clone())))
        }
    }

    unsafe fn open_created_paired_excerpt(&mut self, card: i64, board: i64, title: &str) -> Result<()> {
        unsafe {
            if board == self.board_id() { self.refresh_board()?; }
            self.open_import(card, &format!("已在「{title}」建立跨頁摘錄卡。"));
            Ok(())
        }
    }

    pub(in crate::window) unsafe fn open_board_pdf_excerpt_source(&mut self) -> Result<()> {
        unsafe {
            let cards = self.notebook.pdf_excerpt_cards_on_board(self.board_id())?;
            if cards.is_empty() {
                label(self.widgets.status, "這張白板沒有 PDF 摘錄卡。");
                return Ok(());
            }
            let selected = choose(self.hwnd, self.widgets.links, &cards, |(id, title)| {
                format!("{title}（#{id}）")
            });
            if let Some(index) = selected {
                self.open_pdf_excerpt_source(cards[index].0)?;
            }
            Ok(())
        }
    }

    unsafe fn pdf_excerpt_menu(&mut self, action: usize, attachment: i64, page: u32) -> Result<()> {
        unsafe {
            let annotations: Vec<_> = self.notebook.list_pdf_annotations(attachment)?
                .into_iter().filter(|entry| entry.page == page).collect();
            if annotations.is_empty() {
                label(self.widgets.status, "本頁尚無標註。請先保存一則標註。");
                return Ok(());
            }
            let hwnd = self.hwnd;
            let anchor = self.widgets.links;
            let Some(index) = choose(hwnd, anchor, &annotations, |entry| {
                let summary = if entry.quote.trim().is_empty() { &entry.comment } else { &entry.quote };
                let first: String = summary.lines().next().unwrap_or("").chars().take(48).collect();
                format!("第 {} 頁（#{}） · {first}", entry.page, entry.id)
            }) else { return Ok(()); };
            let annotation = annotations[index].id;
            if action == 9 {
                let boards = self.notebook.list_boards()?;
                let Some(index) = choose(hwnd, anchor, &boards, |board| board.title.clone())
                    else { return Ok(()); };
                let board = &boards[index];
                let note = self.notebook.create_pdf_excerpt_card(board.id, annotation)?;
                if board.id == self.board_id() {
                    self.refresh_board()?;
                }
                self.open_import(note, &format!("已在「{}」建立 PDF 摘錄卡。", board.title));
            } else {
                let cards = self.notebook.pdf_excerpt_cards_for(annotation)?;
                if cards.is_empty() {
                    label(self.widgets.status, "這則標註尚無可開啟的摘錄卡。");
                    return Ok(());
                }
                if let Some(index) = choose(hwnd, anchor, &cards, |(id, title)| format!("{title}（#{id}）")) {
                    self.open_anchored_note(cards[index].0, None)?;
                }
            }
            Ok(())
        }
    }

    pub(super) unsafe fn pdf_menu(&mut self) -> Result<()> {
        unsafe {
            let Some(WorkspaceView::Pdf(view)) = &self.extras.workspace else { return Ok(()); };
            let (attachment, page) = (view.attachment, view.page);
            let count = view.raster.as_ref().map(|frame| frame.page_count as u32);
            if view.adjacent.is_some() {
                return self.pdf_adjacent_menu();
            }
            let source = Arc::clone(&view.source);
            let current_annotation = view.annotation;
            let paired_excerpt = view.paired_excerpt;
            let has_peer = if let Some(annotation) = current_annotation {
                self.notebook.pdf_adjacent_peer(attachment, annotation)?.is_some()
            } else {
                false
            };
            let mut items = vec![
                (1, "上一頁"), (2, "下一頁"), (3, "跳至頁碼…"), (4, "檢視本頁標註"),
                (5, "返回已保存的標註…"),
            ];
            if view.raster.as_ref().is_some_and(|frame| !frame.text.is_empty()) {
                items.push((6, if view.text_mode { "切回矩形選區" } else { "選取 PDF 文字" }));
            }
            let selection = view.text_selection.clone();
            if selection.is_some() {
                items.extend([(7, "複製所選文字"), (8, "保存文字標註…")]);
            }
            items.extend([
                (9, "從本頁標註建立摘錄卡…"),
                (10, "開啟本頁標註的摘錄卡…"),
            ]);
            if selection.is_some() && count.is_some_and(|count| page < count) {
                items.push((11, "跨頁選取並前往下一頁"));
            }
            if has_peer {
                if paired_excerpt.is_none() {
                    items.push((15, "跳到這組標註的另一頁"));
                }
                items.push((16, "從成對標註建立跨頁摘錄卡…"));
            }
            if paired_excerpt.is_some() && view.pending.is_none() {
                items.push((18, "開啟跨頁摘錄卡的另一頁"));
            }
            let action = popup(self.hwnd, self.widgets.links, None, &items);
            match action {
                9 | 10 => return self.pdf_excerpt_menu(action, attachment, page),
                11 => {
                    if let Some(selection) = selection {
                        let draft = AdjacentDraft { source, page, selection };
                        self.open_pdf_view_at(attachment, page + 1, Vec::new(), false, None, Some(draft))?;
                    }
                    return Ok(());
                }
                15 => {
                    let annotation = current_annotation.ok_or("目前沒有開啟的跨頁標註。")?;
                    let (peer, peer_page) = self.notebook.pdf_adjacent_peer(attachment, annotation)?
                        .ok_or("另一頁的標註已移除。")?;
                    return self.open_pdf_annotation_at(attachment, peer_page, peer);
                }
                16 => {
                    let annotation = current_annotation.ok_or("目前沒有開啟的跨頁標註。")?;
                    let (peer, _) = self.notebook.pdf_adjacent_peer(attachment, annotation)?
                        .ok_or("另一頁的標註已移除。")?;
                    let Some((board, title)) = self.choose_pdf_excerpt_board()? else { return Ok(()) };
                    let card = self.notebook.create_paired_pdf_excerpt_card(board, annotation, peer)?;
                    return self.open_created_paired_excerpt(card, board, &title);
                }
                18 => {
                    if let Some(card) = paired_excerpt
                        && !self.open_paired_pdf_excerpt_page(card, Some(page))?
                    {
                        return Err("跨頁摘錄卡目前無法開啟。".into());
                    }
                    return Ok(());
                }
                6 => {
                    if let Some(WorkspaceView::Pdf(view)) = &mut self.extras.workspace {
                        view.text_mode = !view.text_mode;
                        view.text_selection = None;
                        let status = if view.text_mode {
                            "拖曳選取同欄文字，或選取閱讀順序明確的同頁雙欄文字。"
                        } else { "已切回矩形選區。" };
                        label(self.widgets.status, status);
                        InvalidateRect(self.hwnd, &self.board_area(), 0);
                    }
                    return Ok(());
                }
                7 | 8 => {
                    let Some(selection) = selection else { return Ok(()); };
                    if action == 7 {
                        copy_unicode_text(self.hwnd, &selection.quote)?;
                        label(self.widgets.status, "已複製所選 PDF 文字。");
                        return Ok(());
                    }
                    let Some(comment) = crate::dialog::prompt_multiline(
                        self.hwnd, "PDF 文字標註", "註解（可留空）", "", 4096,
                    )? else { return Ok(()); };
                    self.notebook.add_pdf_text_from_source(
                        attachment, source.as_slice(),
                        PdfPageText { page, quote: &selection.quote, regions: &selection.regions }, &comment,
                    )?;
                    if let Some(WorkspaceView::Pdf(view)) = &mut self.extras.workspace
                        && view.attachment == attachment && view.page == page
                    {
                        view.text_marks.extend_from_slice(&selection.regions);
                        view.marked_bgra = view.raster.as_ref()
                            .and_then(|raster| raster.highlighted_bgra(&view.text_marks));
                        view.text_selection = None;
                        InvalidateRect(self.hwnd, &self.board_area(), 0);
                    }
                    label(self.widgets.status, "已保存文字標註。");
                    return Ok(());
                }
                _ => {}
            }
            if action == 5 {
                return self.jump_pdf_annotation(attachment, self.widgets.links);
            }
            if action == 4 {
                return self.show_pdf_page_annotations(attachment, page);
            }
            let Some(count) = count else {
                return if action == 0 { Ok(()) } else { Err("PDF 頁面尚未載入。".into()) };
            };
            let next = match action {
                1 => page.saturating_sub(1).max(1),
                2 => page.saturating_add(1).min(count),
                3 => {
                    let Some(value) = crate::dialog::prompt(
                        self.hwnd, "跳至 PDF 頁碼", "頁碼", &page.to_string(), 8,
                    )? else { return Ok(()); };
                    let next = value.trim().parse::<u32>()?;
                    if !(1..=count).contains(&next) { return Err("PDF 中沒有此頁碼。".into()); }
                    next
                }
                _ => return Ok(()),
            };
            if next != page { self.open_pdf_view(attachment, next)?; }
            Ok(())
        }
    }

    unsafe fn show_pdf_page_annotations(&self, attachment: i64, page: u32) -> Result<()> {
        let mut text = String::new();
        for entry in self.notebook.list_pdf_annotations(attachment)?.into_iter().filter(|item| item.page == page) {
            writeln!(text, "{}\r\n{}\r\n", entry.quote, entry.comment)?;
        }
        if text.is_empty() {
            text.push_str("本頁尚無標註。");
        }
        unsafe { crate::dialog::show_text(self.hwnd, "本頁標註", &text) }
    }

    unsafe fn pdf_adjacent_menu(&mut self) -> Result<()> {
        unsafe {
            let Some(WorkspaceView::Pdf(view)) = &self.extras.workspace else { return Ok(()); };
            let Some(draft) = view.adjacent.clone() else { return Ok(()); };
            let next = view.text_selection.clone();
            let (attachment, page) = (view.attachment, view.page);
            let mut items = vec![(4, "檢視本頁標註")];
            if next.is_some() {
                items.extend([(12, "複製兩頁所選文字"), (13, "保存跨頁標註…")]);
                items.push((17, "保存跨頁標註並建立摘錄卡…"));
            }
            items.push((14, "取消跨頁選取"));
            match popup(self.hwnd, self.widgets.links, None, &items) {
                4 => self.show_pdf_page_annotations(attachment, page),
                12 => {
                    let Some(next) = next else { return Ok(()); };
                    let combined = format!("{}\n{}", draft.selection.quote, next.quote);
                    if combined.encode_utf16().count() > 2048
                        || draft.selection.regions.len() + next.regions.len() > PdfRegion::MAX_TEXT_SEGMENTS
                    {
                        return Err("兩頁合計最多 64 行與 2,048 字元。請縮小選區。".into());
                    }
                    copy_unicode_text(self.hwnd, &combined)?;
                    label(self.widgets.status, "已依頁序複製兩頁所選文字。");
                    Ok(())
                }
                13 => {
                    let Some(next) = next else { return Ok(()); };
                    let Some(comment) = crate::dialog::prompt_multiline(
                        self.hwnd, "PDF 跨頁文字標註", "註解會保存於兩頁，可留空", "", 4096,
                    )? else { return Ok(()); };
                    let (_, next_id) = self.notebook.add_pdf_adjacent_text(
                        attachment, draft.source.as_slice(),
                        PdfPageText { page: draft.page, quote: &draft.selection.quote, regions: &draft.selection.regions },
                        PdfPageText { page, quote: &next.quote, regions: &next.regions }, &comment,
                    )?;
                    let area = self.board_area();
                    if let Some(WorkspaceView::Pdf(view)) = &mut self.extras.workspace {
                        view.annotation = Some(next_id);
                        view.adjacent = None;
                        view.text_selection = None;
                        view.focus = next.regions;
                        view.text_marks.extend_from_slice(&view.focus);
                        view.marked_bgra = view.raster.as_ref()
                            .and_then(|raster| raster.highlighted_bgra(&view.text_marks));
                    }
                    InvalidateRect(self.hwnd, &area, 0);
                    label(self.widgets.status, "已保存兩頁文字標註。可從 PDF 操作跳到另一頁。");
                    Ok(())
                }
                17 => {
                    let Some(next) = next else { return Ok(()); };
                    let Some(comment) = crate::dialog::prompt_multiline(
                        self.hwnd, "PDF 跨頁文字標註", "註解會保存於兩頁，可留空", "", 4096,
                    )? else { return Ok(()); };
                    let Some((board, title)) = self.choose_pdf_excerpt_board()? else { return Ok(()) };
                    let card = self.notebook.capture_paired_pdf_excerpt_card(
                        board, attachment, draft.source.as_slice(), [
                            PdfPageText { page: draft.page, quote: &draft.selection.quote, regions: &draft.selection.regions },
                            PdfPageText { page, quote: &next.quote, regions: &next.regions },
                        ], &comment,
                    )?;
                    self.open_created_paired_excerpt(card, board, &title)
                }
                14 => {
                    let area = self.board_area();
                    if let Some(WorkspaceView::Pdf(view)) = &mut self.extras.workspace {
                        view.adjacent = None;
                        view.text_selection = None;
                    }
                    InvalidateRect(self.hwnd, &area, 0);
                    label(self.widgets.status, "已取消跨頁選取。");
                    Ok(())
                }
                _ => Ok(()),
            }
        }
    }

    pub(super) unsafe fn pdf_input(&mut self, msg: u32, lp: LPARAM) -> Result<()> {
        unsafe {
            let area = self.board_area();
            let inset = self.scaled(12);
            let point = [lp as u16 as i16 as i32, (lp >> 16) as u16 as i16 as i32];
            let Some(WorkspaceView::Pdf(view)) = &mut self.extras.workspace else { return Ok(()); };
            let mut completed = None;
            match msg {
                WM_LBUTTONDOWN => {
                    if let Some(page_box) = view.page_box(area, inset).filter(|page_box| {
                        view.painted.get() == Some(*page_box) && page_box.contains(point)
                    }) {
                        SetCapture(self.hwnd);
                        if GetCapture() == self.hwnd {
                            if view.text_selection.take().is_some() {
                                InvalidateRect(self.hwnd, &area, 0);
                            }
                            view.drag = Some(Drag { page_box, start: point, end: point });
                        }
                    }
                }
                WM_MOUSEMOVE => {
                    if let Some(drag) = &mut view.drag {
                        drag.end = point;
                        InvalidateRect(self.hwnd, &area, 0);
                    }
                }
                WM_LBUTTONUP => {
                    if let Some(mut drag) = view.drag.take() {
                        ReleaseCapture();
                        InvalidateRect(self.hwnd, &area, 0);
                        if view.page_box(area, inset) == Some(drag.page_box)
                            && view.painted.get() == Some(drag.page_box)
                        {
                            if view.text_mode {
                                drag.end = point;
                                view.text_selection = view.raster.as_ref()
                                    .and_then(|frame| drag.select_text(frame));
                                let status = if view.text_selection.is_some() {
                                    "已選取 PDF 文字。可從 PDF 操作複製或保存。"
                                } else {
                                    "未選到可辨識且順序明確的文字，可改用矩形選區。"
                                };
                                label(self.widgets.status, status);
                            } else if let Some(region) = drag.page_box.selection(drag.start, point) {
                                completed = Some((view.attachment, Arc::clone(&view.source), view.page, region));
                            }
                        }
                    }
                }
                WM_CAPTURECHANGED => {
                    if view.drag.take().is_some() { InvalidateRect(self.hwnd, &area, 0); }
                }
                _ => {}
            }
            let Some((attachment, source, page, region)) = completed else { return Ok(()); };
            let Some(quote) = crate::dialog::prompt_multiline(
                self.hwnd, "PDF 選區標註", "引文（可留空）", "", 2048,
            )? else { return Ok(()); };
            let Some(comment) = crate::dialog::prompt_multiline(
                self.hwnd, "PDF 選區標註", "註解（可留空）", "", 4096,
            )? else { return Ok(()); };
            self.notebook.add_pdf_area_from_source(attachment, source.as_slice(), page, &quote, &comment, region)?;
            if let Some(WorkspaceView::Pdf(view)) = &mut self.extras.workspace
                && view.attachment == attachment && view.page == page
            {
                view.regions.push(region);
                InvalidateRect(self.hwnd, &area, 0);
            }
            label(self.widgets.status, "已保存選區標註。");
            Ok(())
        }
    }

    pub(super) unsafe fn paint_pdf(&self, dc: HDC, area: RECT, view: &PdfView) {
        unsafe {
            let Some(frame) = &view.raster else {
                let mut message_box = area;
                message_box.left += self.scaled(12);
                message_box.top += self.scaled(12);
                draw_text(dc, self.fonts[4], SOFT, view.error.as_deref().unwrap_or("正在讀取 PDF 頁面…"),
                          &mut message_box, DT_WORDBREAK);
                return;
            };
            let Some(page_box) = view.page_box(area, self.scaled(12)) else {
                view.painted.set(None);
                return;
            };
            let mut page_rect = RECT {
                left: page_box.x, top: page_box.y,
                right: page_box.x + page_box.width, bottom: page_box.y + page_box.height,
            };
            if RectVisible(dc, &page_rect) == 0 { return; }

            let mut bitmap: BITMAPINFO = std::mem::zeroed();
            bitmap.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bitmap.bmiHeader.biWidth = i32::from(frame.width);
            bitmap.bmiHeader.biHeight = -i32::from(frame.height);
            bitmap.bmiHeader.biPlanes = 1;
            bitmap.bmiHeader.biBitCount = 32;
            bitmap.bmiHeader.biCompression = BI_RGB;
            SetStretchBltMode(dc, COLORONCOLOR);
            let raster = view.marked_bgra.as_deref().unwrap_or(&frame.bgra);
            let lines = StretchDIBits(dc, page_box.x, page_box.y, page_box.width, page_box.height,
                0, 0, i32::from(frame.width), i32::from(frame.height), raster.as_ptr().cast(),
                &bitmap, DIB_RGB_COLORS, SRCCOPY);
            view.painted.set((lines > 0).then_some(page_box));
            if lines <= 0 {
                draw_text(dc, self.fonts[4], SOFT, "無法將 PDF 頁面顯示在視窗。", &mut page_rect, DT_WORDBREAK);
                return;
            }

            let dragging = (!view.text_mode).then_some(view.drag).flatten().and_then(|drag| {
                (drag.page_box == page_box).then(|| page_box.selection(drag.start, drag.end)).flatten()
            });
            let outline = CreateSolidBrush(TEAL);
            if outline.is_null() { return; }
            for region in view.regions.iter().copied().chain(dragging) {
                let [left, top, right, bottom] = page_box.rectangle(region);
                FrameRect(dc, &RECT { left, top, right, bottom }, outline);
            }
            DeleteObject(outline);
            let text_drag = view.drag
                .filter(|drag| view.text_mode && drag.page_box == page_box)
                .and_then(|drag| drag.select_text(frame));
            if let Some(selection) = text_drag.as_ref().or(view.text_selection.as_ref()) {
                let underline = CreateSolidBrush(TEAL);
                if !underline.is_null() {
                    for region in &selection.regions {
                        let [left, _, right, bottom] = page_box.rectangle(*region);
                        FillRect(dc, &RECT { left, top: bottom, right, bottom: bottom + self.scaled(2) }, underline);
                    }
                    DeleteObject(underline);
                }
            }
            if !view.focus.is_empty() {
                let highlight = CreateSolidBrush(AMBER);
                if !highlight.is_null() {
                    for region in &view.focus {
                        let [left, top, right, bottom] = page_box.rectangle(*region);
                        FrameRect(dc, &RECT { left, top, right, bottom }, highlight);
                    }
                    DeleteObject(highlight);
                }
            }
        }
    }
}
