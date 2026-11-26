use super::{*, collection_group_ui::CollectionLine};
use crate::store::collection_display::MAX_DISPLAY_COLUMNS;
use std::collections::HashMap;

pub(super) struct DisplayProperty {
    pub(super) key: String,
    pub(super) values: HashMap<i64, String>,
}

impl DisplayProperty {
    fn value(&self, note: i64) -> &str {
        self.values.get(&note).map_or("", String::as_str)
    }
}

impl App {
    pub(super) unsafe fn collection_display_menu(&mut self, id: i64, title: String) -> Result<()> {
        unsafe {
            let action = popup(self.hwnd, self.widgets.links, None, &[
                (1, "檢視可用屬性…"), (2, "設定顯示順序…"),
                (3, "恢復原有欄位"), (4, "查看資料列的顯示欄位…"),
                (5,"編輯連結的 status…"),(6,"編輯連結的布林或數值…"),
            ]);
            match action {
                1 => {
                    let Some(query) = crate::dialog::prompt(self.hwnd, "可用的集合屬性",
                        "輸入屬性名稱的一部分。留白會列出前 60 個。", "", MAX_KEY_BYTES)?
                    else { return Ok(()) };
                    let (keys, more) = self.notebook.collection_display_keys(id, query.trim())?;
                    let mut report = keys.join("\r\n");
                    if keys.is_empty() { report.push_str("找不到相符的屬性。"); }
                    if more { report.push_str("\r\n\r\n另有欄位。可縮小搜尋範圍。"); }
                    crate::dialog::show_text(self.hwnd, "可用的集合屬性", &report)
                }
                2 | 3 => {
                    let current = self.notebook.collection_display_columns(id)?;
                    let next = if action == 3 {
                        None
                    } else {
                        let initial = current.as_ref().map(|keys| keys.join("\n")).unwrap_or_default();
                        let mode = if current.is_some() { "目前使用自訂欄位。" } else { "目前使用原有欄位。" };
                        let hint = format!("{mode}依顯示順序每行輸入一個現有屬性，最多 {MAX_DISPLAY_COLUMNS} 個。留白只顯示標題與狀態。");
                        let Some(draft) = crate::dialog::prompt_multiline(self.hwnd, "集合欄位顯示",
                            &hint, &initial, MAX_DISPLAY_COLUMNS * (MAX_KEY_BYTES + 2))?
                        else { return Ok(()) };
                        let keys: Vec<_> = draft.lines().filter(|line| !line.trim().is_empty())
                            .map(|line| line.trim().to_owned()).collect();
                        Some((draft, keys))
                    };
                    let new_keys = next.as_ref().map(|(_, keys)| keys.as_slice());
                    if let Err(error) = self.notebook.set_collection_display_columns(id, current.as_deref(), new_keys) {
                        if let Some((draft, _)) = next {
                            return crate::dialog::show_text(self.hwnd, "欄位顯示未儲存（可複製）",
                                &format!("{error}\r\n\r\n{draft}"));
                        }
                        return Err(error);
                    }
                    let saved = self.active_collection_view(id);
                    self.open_collection_layout(id, title, saved)?;
                    label(self.widgets.status, "已儲存集合欄位顯示。");
                    Ok(())
                }
                4 => self.show_collection_display_row(id),
                5 | 6 => {
                    let typed=action==6;
                    let keys=if typed{self.notebook.typed_base_keys(id)?}else{
                        self.notebook.live_status_keys(id)?.into_iter().map(|k|(k,String::new())).collect()
                    };
                    if keys.is_empty(){return Err("這個集合沒有此類可編輯欄位。".into());}
                    let Some(k)=choose(self.hwnd,self.widgets.links,&keys,|(label,_)|label.clone()) else{return Ok(())};
                    let (key,kind)=&keys[k];
                    let Some(WorkspaceView::Kanban(view))=&self.extras.workspace else{return Ok(())};
                    let visible=if typed{self.notebook.typed_base_rows(id,key)?}else{self.notebook.editable_status_rows(id,key)?};
                    let rows:Vec<_>=view.expanded_lanes().flat_map(|(_,rows)|rows.iter()).filter(|note|visible.contains_key(&note.id)).map(|note|(note.id,note.title.clone())).collect();
                    if rows.is_empty(){return Err("目前沒有可編輯的來源資料列。".into());}
                    let Some(n)=choose(self.hwnd,self.widgets.links,&rows,|(id,title)|format!("{title} (#{id})")) else{return Ok(())};
                    let note=rows[n].0;
                    let (version,value)=if typed{self.notebook.typed_base_current(id,key,note)?}else{self.notebook.live_status_snapshot(id,key,note)?};
                    let (title_text,hint)=if typed{("編輯工作區型別欄位",if kind=="number"{"輸入十進位數（整數最多 15 位、小數最多 6 位）或 null。"}else{"輸入 true、false 或 null。"})}else{("編輯工作區 status","儲存後，同一篇來源筆記的連結欄位將顯示新值。")};
                    let Some(draft)=crate::dialog::prompt(self.hwnd,title_text,hint,&value,1024)? else{return Ok(())};
                    let outcome=if typed{self.notebook.edit_typed_base(id,key,note,version,&draft)}else{self.notebook.edit_live_status(id,key,note,version,&draft)};
                    if let Err(e)=outcome{
                        return crate::dialog::show_text(self.hwnd,"欄位未儲存（可複製）",&format!("{e}\r\n\r\n{draft}"));
                    }
                    if !self.dirty && !self.failed && self.current.as_ref().is_some_and(|current|current.row.id==note) {
                        self.current=self.notebook.note(note)?;
                    }
                    let saved=self.active_collection_view(id);
                    self.open_collection_layout(id,title,saved)?;
                    Ok(())
                }
                _ => Ok(()),
            }
        }
    }

    unsafe fn show_collection_display_row(&self, id: i64) -> Result<()> {
        let view = match &self.extras.workspace {
            Some(WorkspaceView::Kanban(view)) if view.id == id && view.is_table()
                && view.calendar.is_none() && view.week.is_none() && view.timeline.is_none() => view,
            _ => return Err("請先開啟這個集合的表格或清單。".into()),
        };
        let rows: Vec<_> = view.expanded_lanes().flat_map(|(group, rows)| rows.iter().map(move |note| (group, note))).collect();
        let chosen = unsafe { choose(self.hwnd, self.widgets.links, &rows,
            |(group, note)| format!("{}　{}  ({})", group.label(), note.title, note.id)) };
        let Some((group, note)) = chosen.map(|index| rows[index]) else { return Ok(()) };
        let group_key = view.saved_view.as_ref().and_then(|saved| saved.presentation.group_key.as_deref()).unwrap_or("狀態");
        let mut report = format!("筆記標題　{}\r\n{group_key}　{}", note.title, group.label());
        if let Some(columns) = &view.display_columns {
            for column in columns {
                write!(report, "\r\n{}　{}", column.key, column.value(note.id))?;
            }
        } else if let Some((key, values)) = &view.checkbox {
            write!(report, "\r\n{key}　{}", collection_view_ui::checkbox_label(values.get(&note.id).map(String::as_str)))?;
        }
        unsafe { crate::dialog::show_text(self.hwnd, "資料列的顯示欄位", &report) }
    }

    pub(super) unsafe fn paint_collection_display_columns(
        &self, dc: HDC, area: RECT, view: &Kanban, columns: &[DisplayProperty],
    ) {
        let (left, right, top, step) = self.collection_table_rect(area);
        let width = right - left;
        if width <= 0 || area.bottom <= top { return; }
        let primary_width = if columns.is_empty() { width } else { width * 45 / 100 };
        let group_width = if view.is_list() { 0 } else { primary_width / 3 };
        let field_left = left + primary_width;
        let padding = self.scaled(6);
        let group_key = view.saved_view.as_ref().and_then(|saved| saved.presentation.group_key.as_deref()).unwrap_or("狀態");
        let list_header = format!("筆記標題／{group_key}");
        let draw_row = |y, item: Option<(&str,&Row)>| {
            let rect = RECT { left, top: y, right, bottom: (y + step - 1).min(area.bottom) };
            unsafe {
                fill(dc, &rect, if item.is_none() { MINT } else { PAPER });
            }
            let cell = |from, to, ink, value: &str| {
                if from + padding >= to { return; }
                let mut rect = RECT { left: from + padding, right: to, ..rect };
                let line = value.lines().next().unwrap_or("");
                unsafe { draw_text(dc, self.fonts[4], ink, line, &mut rect, DT_SINGLELINE | DT_END_ELLIPSIS); }
            };
            let (group, title) = item.map_or((group_key, "筆記標題"), |(group, note)| (group, note.title.as_str()));
            if view.is_list() {
                let content = item.map_or_else(|| list_header.clone(), |_| format!("{title}　·　{group}"));
                cell(left, field_left, INK, &content);
            } else {
                cell(left, left + group_width, TEAL, group);
                cell(left + group_width, field_left, INK, title);
            }
            for (index, column) in columns.iter().enumerate() {
                let boundary = |i| field_left + (width - primary_width) * i as i32 / columns.len() as i32;
                let value = item.map_or(column.key.as_str(), |(_, note)| column.value(note.id));
                cell(boundary(index), boundary(index + 1), if item.is_none() { TEAL } else { INK }, value);
            }
        };
        draw_row(top, None);
        let first = top + step;
        let visible = ((area.bottom - first) / step).max(0) as usize;
        if visible > 0 && view.lanes.is_empty() {
            let mut rect = RECT { left: left + padding, top: first, right, bottom: first + step };
            unsafe { draw_text(dc, self.fonts[4], TEAL, "沒有符合條件的資料列。", &mut rect, DT_SINGLELINE); }
        }
        for (offset, line) in view.table_lines().skip(view.row).take(visible).enumerate() {
            let y = first + offset as i32 * step;
            match line {
                CollectionLine::Heading(group, count, folded) => {
                    let rect = RECT { left, top: y, right, bottom: y + step - 1 };
                    unsafe { self.paint_collection_fold_header(dc, rect, group, count, folded); }
                }
                CollectionLine::Note(group, note) => draw_row(y, Some((&group.label(), note))),
            }
        }
    }
}
