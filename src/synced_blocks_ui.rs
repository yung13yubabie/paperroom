use super::*;
use super::navigation_ui::{choose_navigation, choose_navigation_page};

impl App {
    pub(super) unsafe fn insert_synced_block(&mut self) -> Result<()> {
        unsafe {
            let (host, version) = self.sync_host()?;
            let Some(query) = crate::dialog::prompt(
                self.hwnd, "尋找同步區塊來源", "輸入來源筆記的標題或路徑。", "", 4096,
            )? else { return Ok(()) };
            let hits = self.notebook.quick_notes(&query)?;
            let sources: Vec<_> = hits.iter().filter(|hit| hit.id != host).collect();
            if sources.is_empty() {
                label(self.widgets.status, "找不到其他符合條件的來源筆記。");
                return Ok(());
            }
            let Some(index) = choose_navigation_page(self.hwnd, self.widgets.files, sources.len(), |i| &sources[i].label)
                else { return Ok(()) };
            let Some(marker) = self.pick_sync_marker(sources[index].id)? else { return Ok(()) };
            self.notebook.append_synced_block(host, version, &marker)?;
            self.show_saved_sync_host(host, "已將同步區塊加入本篇末尾。")?;
            Ok(())
        }
    }

    pub(super) unsafe fn manage_synced_blocks(&mut self) -> Result<()> {
        unsafe {
            let (host, _) = self.sync_host()?;
            let (blocks, limited) = self.notebook.synchronized_blocks(host)?;
            if limited {
                label(self.widgets.status, "已達同步區塊掃描上限，只顯示已成功載入的位置。");
            }
            if blocks.is_empty() && limited {
                return Ok(());
            }
            if blocks.is_empty() {
                label(self.widgets.status, "本篇沒有可管理的同步區塊。請檢查是否仍有有效的原文及唯一 ID。");
                return Ok(());
            }
            let Some(index) = choose_navigation_page(self.hwnd, self.widgets.files, blocks.len(), |i| {
                let hit = &blocks[i];
                let excerpt: String = hit.block.lines().find(|line| !line.trim().is_empty()).unwrap_or("")
                    .trim().chars().take(48).collect();
                format!("第 {} 行　{}　{}　{}", hit.line, hit.title, hit.anchor, excerpt)
            }) else { return Ok(()) };
            let hit = &blocks[index];
            let actions = ["開啟來源區塊", "編輯共用原文", "解除同步並保留文字"];
            match choose_navigation(self.hwnd, self.widgets.files, &actions) {
                Some(0) => {
                    let (id, version, selected) = self.notebook.synchronized_source(hit)?;
                    self.open_import(id, "已開啟同步區塊來源。");
                    if self.current.as_ref().is_some_and(|note| note.row.id == id && note.version == version) {
                        self.focus_source_selection(selected.start, selected.end)?;
                    }
                }
                Some(1) => {
                    let Some(edited) = crate::dialog::prompt_multiline(self.hwnd, "編輯共用區塊",
                        "這會修改來源筆記，其他引用會顯示新原文。請保留原區塊的行數結構。", &hit.block, 256 * 1024,
                    )? else { return Ok(()) };
                    if let Err(error) = self.notebook.update_synced_block(hit, &edited) {
                        crate::dialog::show_text(self.hwnd, "未儲存的共用原文", &edited)?;
                        return Err(error);
                    }
                    self.show_saved_sync_host(host, "已儲存共用原文。")?;
                }
                Some(2) => {
                    if message(self.hwnd, "解除同步並保留文字",
                        &format!("把第 {} 行的引用改為目前的文字？其他引用會繼續同步。", hit.line),
                        MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES
                    {
                        self.notebook.detach_synced_block(hit)?;
                        self.show_saved_sync_host(host, "已解除這一處同步。")?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }

    pub(super) unsafe fn view_synced_consumers(&mut self) -> Result<()> {
        unsafe {
            let (source, _) = self.sync_host()?;
            let Some(marker) = self.pick_sync_marker(source)? else { return Ok(()) };
            let (consumers, limited) = self.notebook.synced_consumers(source, &marker)?;
            if limited {
                label(self.widgets.status, "只顯示掃描範圍內的同步引用。");
            } else if consumers.is_empty() {
                label(self.widgets.status, "沒有使用中的筆記引用此同步區塊。");
            }
            if !consumers.is_empty() {
                if let Some(i) = choose_navigation_page(self.hwnd, self.widgets.files, consumers.len(), |i| &consumers[i].label) {
                    self.open_import(consumers[i].note, "已開啟引用此同步區塊的筆記。");
                }
            }
            Ok(())
        }
    }

    fn sync_host(&self) -> Result<(i64, i64)> {
        self.current.as_ref().filter(|note| note.row.trash_root.is_none() && !self.extras.board)
            .map(|note| (note.row.id, note.version))
            .ok_or_else(|| "請先開啟一篇使用中的筆記。".into())
    }

    unsafe fn pick_sync_marker(&self, source: i64) -> Result<Option<String>> {
        unsafe {
            let Some(query) = crate::dialog::prompt(
                self.hwnd, "尋找同步區塊 ID", "輸入區塊文字或 ID，留白列出已標記且唯一的區塊。", "", 4096,
            )? else { return Ok(None) };
            let (hits, limited) = self.notebook.block_link_candidates(source, false, &query)?;
            if hits.is_empty() {
                label(self.widgets.status, "沒有符合條件的唯一區塊。先在來源 Markdown 新增 ^block-id。");
                return Ok(None);
            }
            if limited {
                label(self.widgets.status, "只顯示前 200 個符合條件的區塊。");
            }
            let Some(index) = choose_navigation_page(self.hwnd, self.widgets.files, hits.len(), |i| &hits[i].label)
                else { return Ok(None) };
            Ok(Some(format!("!{}", self.notebook.stable_block_link(&hits[index])?)))
        }
    }

    unsafe fn show_saved_sync_host(&mut self, id: i64, status: &str) -> Result<()> {
        // The current tree selection is already this note. Load the saved
        // snapshot explicitly because select() skips the current ID.
        self.current = self.notebook.note(id)?;
        self.editing = false;
        unsafe {
            self.present();
            label(self.widgets.status, status);
        }
        Ok(())
    }
}
