use super::*;
use crate::store::named_relation_navigation::{Direction, PAGE_SIZE};

impl App {
    pub(super) unsafe fn show_named_relation_navigation(&mut self, note: i64) -> Result<()> {
        let origin = self.notebook.named_relation_origin(note)?;
        let anchor = self.widgets.links;
        let direction = match unsafe { popup(self.hwnd, anchor, None, &[
            (1, "指向此筆記的具名關聯…"),
            (2, "此筆記指向的具名關聯…"),
        ]) } {
            1 => Direction::Incoming,
            2 => Direction::Outgoing,
            _ => return Ok(()),
        };
        let mut page = 0;
        loop {
            let (hits, more) = self.notebook.named_relation_page(&origin, direction, page)?;
            if page == 0 && hits.is_empty() {
                unsafe { label(self.widgets.status, "沒有可開啟的具名關聯。"); }
                return Ok(());
            }
            let labels: Vec<_> = hits.iter().map(|hit| {
                let location = hit.location.as_deref().unwrap_or("工作區根層");
                format!("{}　{}　｜ {}　[#{id}]", hit.name, hit.title, location, id = hit.note)
            }).collect();
            let mut items: Vec<_> = labels.iter().enumerate()
                .map(|(i, label)| (i + 1, label.as_str())).collect();
            let previous = PAGE_SIZE + 1;
            let next = PAGE_SIZE + 2;
            if page > 0 { items.push((previous, "上一頁")); }
            if more { items.push((next, "下一頁")); }
            match unsafe { popup(self.hwnd, anchor, None, &items) } {
                command if command == previous => page -= 1,
                command if command == next => page += 1,
                command if (1..=hits.len()).contains(&command) => {
                    let target = self.notebook.note_from_named_relation(&origin, direction, &hits[command - 1])?;
                    unsafe {
                        self.view = View::All;
                        self.editing = false;
                        label(self.widgets.search, "");
                        self.set_current_note(Some(target));
                        self.rebuild();
                        label(self.widgets.status, "已開啟具名關聯的筆記。");
                    }
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }
    }
}

impl App {
    pub(super) unsafe fn bind_notion_source_page(&mut self, row: i64) -> Result<()> {
        let token = self.notebook.notion_row_token(row)?.ok_or("此筆記已經不在集合中。")?;
        let batches = self.notebook.notion_source_batches()?;
        if batches.is_empty() { return Err("找不到含 Markdown 來源頁的 Notion 匯入批次。".into()); }
        let anchor = self.widgets.links;
        let Some(index) = (unsafe { navigation_ui::choose_navigation_page(self.hwnd, anchor, batches.len(),
            |i| format!("{}  (#{})", batches[i].title,batches[i].id)) }) else { return Ok(()); };
        let batch = &batches[index];
        let pages = self.notebook.notion_source_pages(batch,row)?;
        if pages.is_empty() { return Err("此批次沒有其他可連結的來源頁。".into()); }
        let Some(index) = (unsafe { navigation_ui::choose_navigation_page(self.hwnd, anchor, pages.len(),
            |i| pages[i].label()) }) else { return Ok(()); };
        let page = &pages[index];
        let prompt = format!("將集合資料列 [#{row}] 連到您選取的來源頁？\r\n{}  (#{})\r\n{}\r\n這會建立本機具名關聯。",batch.title,batch.id,page.label());
        if unsafe { message(self.hwnd,"連結 Notion 來源頁",&prompt,MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) } == IDYES {
            self.notebook.bind_notion_source(row,&token,batch,page)?;
            unsafe { label(self.widgets.status,"已連結 Notion Markdown 來源頁。"); }
        }
        Ok(())
    }
}
