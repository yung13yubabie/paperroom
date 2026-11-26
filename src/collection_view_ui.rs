use super::*;
use std::collections::BTreeMap;
use crate::store::collection_views::{
    CalendarDay, ChildDraft, CollectionLayout, CollectionTemplate, CollectionView, FieldEdit, FilterGroup, FilterKind, FilterMode, FilterOperator, FilterRule, MAX_FILTERS, MAX_FILTER_GROUP_DEPTH, MAX_FILTER_GROUPS, Month,
    Presentation, PropertyFilter, RelationMember, ScheduleAdjustment, ScheduledRow, SortRule, SortSource, TemplateApply, TemplateChild, TemplateField, TemplateSelection, validate_template_body,
};
use crate::store::typed_properties::PropertyType;
use super::{collection_display_ui::DisplayProperty, collection_group_ui::CollectionLine};

#[derive(Clone, Copy)]
struct FilterRoom {
    leaves: usize,
    groups: usize,
}

impl FilterRoom {
    const FULL: Self = Self { leaves: MAX_FILTERS, groups: MAX_FILTER_GROUPS };

    fn without(self, (leaves, groups): (usize, usize)) -> Self {
        Self { leaves: self.leaves - leaves, groups: self.groups - groups }
    }

    fn holds(self, (leaves, groups): (usize, usize)) -> bool {
        leaves <= self.leaves && groups <= self.groups
    }
}

const FILTER_LIMIT: &str = "每個檢視最多可有 16 個篩選條件及 16 個群組。";

pub(super) struct TimelineDrag {
    note: i64,
    month: Month,
    grid: [i32; 4],
    press: [i32; 2],
    current: [i32; 2],
    threshold: i32,
    moved: bool,
    adjustment: Option<ScheduleAdjustment>,
    expected: [Option<String>; 2],
    editable: bool,
}

impl TimelineDrag {
    fn update(&mut self, point: [i32; 2]) {
        self.current = point;
        self.moved |= (self.current[0] - self.press[0]).abs().max((self.current[1] - self.press[1]).abs()) >= self.threshold;
    }

    fn delta(&self) -> i8 {
        let pixels = self.current[0] - self.press[0];
        let width = self.grid[2] - self.grid[0];
        let days = self.month.days() as i32;
        let delta = ((pixels.abs() * days + width / 2) / width).min(days);
        (pixels.signum() * delta) as i8
    }

    fn dates(&self) -> Option<(CalendarDay, Option<CalendarDay>)> {
        if !self.editable { return None; }
        let start = CalendarDay::parse(self.expected[0].as_deref()?)?;
        let end = self.expected[1].as_deref().and_then(CalendarDay::parse);
        self.adjustment?.apply(start, end, self.delta())
    }
}

struct TimelineGrid {
    chart: RECT,
    label_left: i32,
    row_step: i32,
}

impl TimelineGrid {
    fn fingerprint(&self) -> [i32; 4] {
        [self.chart.left, self.chart.top, self.chart.right, self.chart.bottom]
    }

    fn row_at(&self, y: i32) -> Option<usize> {
        (y >= self.chart.top && y < self.row_top(self.row_count()))
            .then(|| ((y - self.chart.top) / self.row_step) as usize)
    }

    fn day_edge(&self, offset: usize, days: usize) -> i32 {
        self.chart.left + (self.chart.right - self.chart.left) * offset as i32 / days as i32
    }

    fn row_count(&self) -> usize {
        ((self.chart.bottom - self.chart.top) / self.row_step) as usize
    }

    fn row_top(&self, row: usize) -> i32 {
        self.chart.top + row as i32 * self.row_step
    }
}

impl Kanban {
    pub(super) fn is_table(&self) -> bool {
        self.saved_view.as_ref().is_some_and(|v| v.layout == CollectionLayout::Table)
    }

    pub(super) fn is_list(&self) -> bool {
        self.saved_view.as_ref().is_some_and(|view|view.presentation.list_mode)
    }

    pub(super) fn first_table_line(&self, top: i32, step: i32) -> i32 {
        if self.is_list() && self.display_columns.is_none() { top } else { top + step }
    }

    pub(super) fn max_scroll_row(&self) -> usize {
        if let Some((_, rows)) = &self.timeline { return rows.len().saturating_sub(1); }
        let count = if self.is_table() { self.table_lines().count() } else {
            self.expanded_lanes().map(|(_, rows)| rows.len()).max().unwrap_or(0)
        };
        count.saturating_sub(1)
    }
}

impl App {
    pub(super) fn active_collection_view(&self, id:i64) -> Option<CollectionView> {
        match &self.extras.workspace {
            Some(WorkspaceView::Kanban(v)) if v.id == id => v.saved_view.clone(),
            _ => None,
        }
    }

    pub(super) fn visible_collection_rows(&self, id:i64) -> Result<Vec<(Row,String)>> {
        let view = self.active_collection_view(id);
        let rows = self.notebook.filtered_collection_rows(id,view.as_ref())?;
        if let Some(WorkspaceView::Kanban(active)) = &self.extras.workspace
            && active.id == id && (active.week.is_some() || active.calendar.is_some() || active.timeline.is_some())
        {
            let mut by_id: std::collections::HashMap<_,_> =
                rows.into_iter().map(|(row,status)| (row.id,(row,status))).collect();
            let ordered = active.week.iter().flat_map(|(_,days)| days.values())
                .chain(active.calendar.iter().flat_map(|(_,days)| days.values())).flatten()
                .chain(active.timeline.iter().flat_map(|(_, rows)| rows.iter().map(|entry| &entry.row)));
            return Ok(ordered.filter_map(|row|by_id.remove(&row.id)).collect());
        }
        let folds = match &self.extras.workspace {
            Some(WorkspaceView::Kanban(active)) if active.id == id => active.group_folds.as_ref(),
            _ => None,
        };
        if folds.is_none() && !view.as_ref().is_some_and(|view| view.presentation.group_key.is_some() || !view.sort_rules().is_empty()) {
            return Ok(rows);
        }
        let mut by_id: std::collections::HashMap<_,_> =
            rows.into_iter().map(|(row,status)| (row.id,(row,status))).collect();
        Ok(self.notebook.presented_collection_lanes(id,view.as_ref())?.into_iter()
            .filter(|(group, _)| !folds.is_some_and(|folds| folds.contains(group)))
            .flat_map(|(_,rows)| rows).filter_map(|row|by_id.remove(&row.id)).collect())
    }

    pub(super) unsafe fn open_collection_layout(&mut self,id:i64,title:String,saved_view:Option<CollectionView>) -> Result<()> {
        let timeline = if let Some(view) = saved_view.as_ref().filter(|view| view.presentation.timeline_mode) {
            let month = view.presentation.calendar_month.ok_or("時間軸的月份有誤。")?;
            Some((month, self.notebook.timeline_collection_rows(id, view, month)?))
        } else { None };
        let week = if let Some(view) = saved_view.as_ref()
            && let Some(start) = view.presentation.calendar_week_day
        {
            let end = start.shifted(6).unwrap_or(CalendarDay::LAST);
            Some((start,self.notebook.calendar_dated_rows(id,view,start,end)?))
        } else { None };
        let calendar = if let Some(view) = saved_view.as_ref()
            .filter(|view| view.presentation.calendar_key.is_some() && week.is_none() && timeline.is_none())
        {
            let month = view.presentation.calendar_month.unwrap_or_else(current_month);
            Some((month,self.notebook.calendar_collection_rows(id,view,month)?))
        } else { None };
        let lanes = if timeline.is_some() || week.is_some() || calendar.is_some() {
            Vec::new()
        } else { self.notebook.presented_collection_lanes(id, saved_view.as_ref())? };
        let (column,row) = match &self.extras.workspace {
            Some(WorkspaceView::Kanban(old)) if old.id == id && old.saved_view == saved_view => (old.column,old.row),
            _ => (0,0),
        };
        let column = column.min(lanes.len().saturating_sub(1));
        let display_columns = if saved_view.as_ref().is_some_and(|view| view.layout == CollectionLayout::Table)
            && timeline.is_none() && week.is_none() && calendar.is_none()
        {
            self.notebook.collection_display_columns(id)?.map(|keys| {
                keys.into_iter().map(|key| {
                    let values = self.notebook.collection_display_values(id, &key)?;
                    Ok(DisplayProperty { key, values })
                }).collect::<Result<Vec<_>>>()
            }).transpose()?
        } else { None };
        let checkbox = if display_columns.is_none() && let Some(filter) = saved_view.as_ref()
            .filter(|view| view.layout == CollectionLayout::Table && view.presentation.group_key.is_none()
                && !view.presentation.list_mode && view.presentation.calendar_key.is_none())
            .map(CollectionView::rules)
            .and_then(|rules| rules.first_checkbox().cloned())
        {
            if self.notebook.property_type(&filter.key)? == Some(PropertyType::Checkbox) {
                Some((filter.key.clone(), self.notebook.collection_property_values(id,&filter.key)?))
            } else {
                None
            }
        } else {
            None
        };
        let group_folds = saved_view.as_ref().map(|view| self.notebook.collection_group_folds(view)).transpose()?.flatten();
        let mut view = Kanban { id,title,saved_view,column,row,checkbox,display_columns,calendar,week,timeline,timeline_drag:None,lanes,group_folds };
        view.row = row.min(view.max_scroll_row());
        unsafe { self.activate_workspace(WorkspaceView::Kanban(view)); }
        Ok(())
    }

    pub(super) unsafe fn collection_view_menu(&mut self,id:i64,title:String) -> Result<()> {
        unsafe {
            let hwnd = self.hwnd;
            let anchor = self.widgets.links;
            let action = popup(hwnd,anchor,None,&[(1,"儲存新檢視…"),(2,"開啟已儲存的檢視…"),
                                                  (3,"顯示全部資料列（看板）"),(4,"刪除已儲存的檢視…"),(5,"儲存含屬性篩選的新檢視…"),
                                                  (6,"儲存日期月曆…"),(7,"儲存分組與排序檢視…"),
                                                  (8,"儲存日期週檢視…"),(9,"限制日期檢視的顯示範圍…"),(10,"儲存清單檢視…"),
                                                  (11,"編輯篩選與多欄排序…"),(12,"設定資料列的結束日期欄…"),
                                                  (13,"重新命名檢視…"),(14,"複製檢視…"),
                                                  (15,"儲存時間軸檢視…"),(16,"將檢視設定套用到其他集合…"),
                                                  (17,"編輯狀態與標題篩選…")]);
            match action {
                1 | 5 => {
                    let Some(name) = crate::dialog::prompt(hwnd,"儲存新檢視","檢視名稱","",256)? else { return Ok(()) };
                    let layout = match popup(hwnd,anchor,None,&[(1,"看板"),(2,"表格")]) {
                        1 => CollectionLayout::Board, 2 => CollectionLayout::Table, _ => return Ok(()),
                    };
                    let Some(status) = crate::dialog::prompt(hwnd,"集合檢視","狀態名稱（完全相符，留白顯示全部）","",256)? else { return Ok(()) };
                    let Some(keyword) = crate::dialog::prompt(hwnd,"集合檢視","標題包含的文字（留白顯示全部）","",256)? else { return Ok(()) };
                    let view = if action == 5 {
                        let defs = self.notebook.property_definitions()?;
                        if defs.is_empty() { return Err("請先在工作區設定屬性型別。".into()); }
                        let Some(index) = choose(hwnd,anchor,&defs,|(k,t)|format!("{k}　{}",t.hint())) else { return Ok(()); };
                        let (key,kind) = &defs[index];
                        let value = if *kind == PropertyType::Checkbox {
                            match popup(hwnd,anchor,None,&[(1,"顯示已勾選"),(2,"顯示未勾選")]) {
                                1 => "true".to_owned(),
                                2 => "false".to_owned(),
                                _ => return Ok(()),
                            }
                        } else {
                            let Some(value) = crate::dialog::prompt(hwnd,"屬性完全相符",kind.hint(),"",MAX_VALUE_BYTES)?
                            else { return Ok(()) };
                            value
                        };
                        let filter = PropertyFilter {key:key.clone(),kind:*kind,value};
                        self.notebook.save_collection_view(id,&name,layout,&status,&keyword,Some(filter))?
                    } else {
                        self.notebook.create_collection_view(id,&name,layout,&status,&keyword)?
                    };
                    self.open_collection_layout(id,title,Some(view))
                }
                6 | 7 | 8 | 10 => {
                    let Some(name) = crate::dialog::prompt(hwnd,"儲存新檢視","檢視名稱","",256)?
                    else { return Ok(()) };
                    let mut presentation = Presentation::default();
                    let layout = if matches!(action,6|8) {
                        let defs = self.notebook.property_definitions()?.into_iter()
                            .filter(|(_,kind)| *kind == PropertyType::Date).map(|(key,_)| key).collect::<Vec<_>>();
                        if defs.is_empty() { return Err("請先設定日期屬性型別。".into()); }
                        let Some(index) = choose(hwnd,anchor,&defs,String::clone) else { return Ok(()) };
                        presentation.calendar_key = Some(defs[index].clone());
                        if action == 8 { presentation.calendar_week_day = Some(current_day()?.monday()); }
                        else { presentation.calendar_month = Some(current_month()); }
                        CollectionLayout::Table
                    } else if action == 10 {
                        let Some(status) = crate::dialog::prompt(hwnd,"集合清單","狀態（留空顯示全部）","",256)? else { return Ok(()) };
                        let Some(keyword) = crate::dialog::prompt(hwnd,"集合清單","標題包含（留空顯示全部）","",256)? else { return Ok(()) };
                        presentation.list_mode = true;
                        let view = self.notebook.save_presented_collection_view(
                            id,&name,CollectionLayout::Table,&status,&keyword,None,presentation,
                        )?;
                        return self.open_collection_layout(id,title,Some(view));
                    } else {
                        let layout = match popup(hwnd,anchor,None,&[(1,"看板"),(2,"表格")]) {
                            1 => CollectionLayout::Board, 2 => CollectionLayout::Table, _ => return Ok(()),
                        };
                        let Some(group) = crate::dialog::prompt(hwnd,"分組","分組屬性名稱（留白依狀態）","",256)?
                        else { return Ok(()) };
                        let Some(sort) = crate::dialog::prompt(hwnd,"排序","排序屬性名稱（留白依筆記標題）","",256)?
                        else { return Ok(()) };
                        presentation.group_key = (!group.trim().is_empty()).then(||group.trim().to_owned());
                        presentation.sort_key = (!sort.trim().is_empty()).then(||sort.trim().to_owned());
                        if presentation.sort_key.is_some() {
                            presentation.sort_desc = match popup(hwnd,anchor,None,&[(1,"升冪"),(2,"降冪")]) {
                                1 => false, 2 => true, _ => return Ok(()),
                            };
                        }
                        layout
                    };
                    let view = self.notebook.save_presented_collection_view(id,&name,layout,"","",None,presentation)?;
                    self.open_collection_layout(id,title,Some(view))
                }
                9 => self.edit_collection_range(id,title),
                11 => self.edit_collection_rules(id,title),
                12 => self.edit_collection_end_key(id,title),
                15 => self.create_collection_timeline(id, title),
                17 => self.edit_collection_basic_filter(id, title),
                2 | 4 | 13 | 14 | 16 => {
                    let views = self.notebook.collection_views(id)?;
                    if views.is_empty() { label(self.widgets.status,"這個集合尚未儲存檢視。"); return Ok(()); }
                    let Some(index) = choose(hwnd,anchor,&views,|v| {
                        let layout = if v.presentation.timeline_mode { "時間軸" }
                            else if v.presentation.calendar_week_day.is_some() { "週曆" }
                            else if v.presentation.calendar_key.is_some() { "月曆" }
                            else if v.presentation.list_mode { "清單" } else if v.layout == CollectionLayout::Table { "表格" } else { "看板" };
                        format!("{}（{}）",v.name,layout)
                    }) else { return Ok(()); };
                    let selected = &views[index];
                    match action {
                        2 => self.open_collection_layout(id,title,Some(selected.clone())),
                        13 | 14 => self.name_collection_view(id, title, selected, action == 14),
                        16 => self.apply_collection_view(id, selected),
                        _ => {
                            let confirm = format!("刪除檢視「{}」",selected.name);
                            if popup(hwnd,anchor,None,&[(1,&confirm),(2,"取消")]) == 1 {
                                self.notebook.delete_collection_view(id,selected.id)?;
                                if self.active_collection_view(id).is_some_and(|v| v.id == selected.id) {
                                    self.open_collection_layout(id,title,None)?;
                                }
                            }
                            Ok(())
                        }
                    }
                }
                3 => self.open_collection_layout(id,title,None),
                _ => Ok(()),
            }
        }
    }

    unsafe fn edit_collection_basic_filter(&mut self, id: i64, title: String) -> Result<()> {
        let view = self.active_collection_view(id).ok_or("請先開啟已儲存的集合檢視。")?;
        unsafe {
            let Some(status) = crate::dialog::prompt(self.hwnd, "編輯集合篩選", "狀態須完全相符。留白會清除狀態條件。",
                &view.status, MAX_KEY_BYTES)? else { return Ok(()) };
            let Some(keyword) = crate::dialog::prompt(self.hwnd, "編輯集合篩選", "標題包含的文字。留白會清除標題條件。",
                &view.keyword, MAX_KEY_BYTES)? else { return Ok(()) };
            self.notebook.set_collection_basic_filter(id, &view, &status, &keyword)?;
            self.reopen_collection_view(id, title, view.id)
        }
    }

    unsafe fn create_collection_timeline(&mut self, id: i64, title: String) -> Result<()> {
        let hw = self.hwnd;
        let anchor = self.widgets.links;
        let mut dates = self.notebook.property_definitions()?.into_iter()
            .filter_map(|(key, kind)| (kind == PropertyType::Date).then_some(key)).collect::<Vec<_>>();
        if dates.is_empty() { return Err("請先設定時間軸使用的日期屬性。".into()); }
        unsafe {
            let Some(name) = crate::dialog::prompt(hw, "儲存時間軸檢視", "檢視名稱", "", MAX_KEY_BYTES)?
            else { return Ok(()) };
            let Some(index) = choose(hw, anchor, &dates, |key| format!("起始日期欄　{key}"))
            else { return Ok(()) };
            let start = dates.remove(index);
            let endings = [None].into_iter().chain(dates.iter().map(|key| Some(key.as_str()))).collect::<Vec<_>>();
            let Some(selected) = choose(hw, anchor, &endings, |key| {
                key.map_or_else(|| "僅使用起始日".to_owned(), |key| format!("結束日期欄　{key}"))
            }) else { return Ok(()) };
            let Some(month) = self.prompt_timeline_month(current_month())? else { return Ok(()) };
            let view = self.notebook.save_presented_collection_view(
                id, &name, CollectionLayout::Table, "", "", None, Presentation {
                    calendar_key: Some(start), calendar_end_key: endings[selected].map(str::to_owned),
                    calendar_month: Some(month), timeline_mode: true, ..Default::default()
                },
            )?;
            self.open_collection_layout(id, title, Some(view))
        }
    }

    unsafe fn apply_collection_view(&mut self, origin: i64, view: &CollectionView) -> Result<()> {
        let targets = self.notebook.collections()?.into_iter().filter(|(id, _)| *id != origin).collect::<Vec<_>>();
        if targets.is_empty() { return Err("請先新增可套用設定的其他集合。".into()); }
        unsafe {
            let Some(index) = choose(self.hwnd, self.widgets.links, &targets,
                |(id, name)| format!("目標集合　{name}  (#{id})")) else { return Ok(()) };
            let (destination, title) = &targets[index];
            let Some(name) = crate::dialog::prompt(self.hwnd, "套用檢視設定", "輸入目標檢視名稱。新檢視會使用目標集合的資料列。",
                &view.name, MAX_KEY_BYTES)? else { return Ok(()) };
            let copy = self.notebook.copy_collection_view_to(origin, view, *destination, &name)?;
            self.reopen_collection_view(*destination, title.clone(), copy)?;
            label(self.widgets.status, "已套用檢視設定到目標集合。");
        }
        Ok(())
    }

    unsafe fn prompt_timeline_month(&self, previous: Month) -> Result<Option<Month>> {
        let input = unsafe { crate::dialog::prompt(self.hwnd, "時間軸月份", "輸入 YYYY-MM。", &previous.iso(), 7)? };
        input.map(|raw| Month::parse(&raw).ok_or_else(|| "請輸入有效的 YYYY-MM。".into())).transpose()
    }

    pub(super) unsafe fn jump_collection_timeline(&mut self, id: i64, title: String) -> Result<()> {
        let view = self.active_collection_view(id).filter(|view| view.presentation.timeline_mode)
            .ok_or("請先開啟時間軸檢視。")?;
        let current = view.presentation.calendar_month.ok_or("時間軸的月份有誤。")?;
        let Some(month) = (unsafe { self.prompt_timeline_month(current)? }) else { return Ok(()) };
        self.notebook.set_collection_calendar_month(id, &view, month)?;
        unsafe { self.reopen_collection_view(id, title, view.id) }
    }

    unsafe fn name_collection_view(&mut self, id: i64, title: String, view: &CollectionView, duplicate: bool) -> Result<()> {
        let (caption, hint, initial) = if duplicate {
            ("複製檢視", "新檢視會沿用設定並使用同一集合的資料列。輸入新名稱。", "")
        } else {
            ("重新命名檢視", "輸入檢視的新名稱。", view.name.as_str())
        };
        let Some(name) = (unsafe { crate::dialog::prompt(self.hwnd, caption, hint, initial, 256)? }) else { return Ok(()) };
        if duplicate {
            let copy = self.notebook.duplicate_collection_view(id, view, &name)?;
            unsafe { self.reopen_collection_view(id, title, copy)?; }
        } else {
            self.notebook.rename_collection_view(id, view, &name)?;
            if self.active_collection_view(id).is_some_and(|active| active.id == view.id) {
                unsafe { self.reopen_collection_view(id, title, view.id)?; }
            }
        }
        unsafe {
            label(self.widgets.status, if duplicate { "已複製檢視設定。" } else { "已重新命名檢視。" });
        }
        Ok(())
    }

    pub(super) unsafe fn collection_template_menu(&mut self, id: i64) -> Result<()> {
        let hwnd = self.hwnd;
        let anchor = self.widgets.links;
        unsafe {
            let action = popup(hwnd, anchor, None, &[
                (1,"由範本新增資料列…"), (2,"儲存資料列為範本…"), (3,"編輯既有範本…"), (4,"移除範本…"),
                (5,"新增空白資料列範本…"),
            ]);
            if action == 2 { return self.capture_collection_template_ui(id); }
            if action == 5 { return self.create_blank_template_ui(id); }
            if !matches!(action, 1 | 3 | 4) { return Ok(()); }
            let templates = self.notebook.collection_templates(id)?;
            if templates.is_empty() { return Err("這個集合尚未儲存資料列範本。".into()); }
            let Some(which) = choose(hwnd, anchor, &templates, |(_, name)| name.clone()) else { return Ok(()) };
            let template = self.notebook.collection_template(id, templates[which].0)?;
            match action {
                1 => {
                    let preview = template_preview(&template)?;
                    crate::dialog::show_text(hwnd, "檢閱要帶入的範本", &preview)?;
                    let include_resources = if template.resources.is_empty() { false } else {
                        match popup(hwnd, anchor, None, &[
                            (1,"套用選定的關聯與附件"),
                            (2,"只套用正文和型別欄位"),
                            (3,"取消整次建立"),
                        ]) {
                            1 => true,
                            2 => false,
                            _ => return Ok(()),
                        }
                    };
                    let include_children = if template.children.is_empty() { false } else {
                        match popup(hwnd, anchor, None, &[
                            (1,"套用選定的子頁"),
                            (2,"省略子頁"),
                            (3,"取消整次建立"),
                        ]) {
                            1 => true,
                            2 => false,
                            _ => return Ok(()),
                        }
                    };
                    let Some(title) = crate::dialog::prompt(hwnd, "由範本新增資料列", "輸入新筆記標題。",
                        &template.default_title, crate::store::MAX_TITLE_CHARS)?
                    else { return Ok(()); };
                    match self.notebook.instantiate_collection_template(id, &template, &title,
                        TemplateApply { resources: include_resources, children: include_children })
                    {
                        Ok(note) => self.open_import(note, "已由範本新增資料列。"),
                        Err(error) => crate::dialog::show_text(hwnd,"範本資料列未儲存（可複製）",
                            &format!("{error}\r\n\r\n{title}"))?,
                    }
                }
                3 => self.edit_collection_template_ui(id, Some(&template), template.clone())?,
                4 => {
                    let label_text = format!("移除「{}」範本", template.name);
                    if popup(hwnd, anchor, None, &[(1,&label_text),(2,"取消")]) == 1 {
                        self.notebook.remove_collection_template(id, &template)?;
                        label(self.widgets.status, "已移除資料列範本。");
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    unsafe fn create_blank_template_ui(&self, id: i64) -> Result<()> {
        unsafe {
            let Some(name) = crate::dialog::prompt(self.hwnd, "新增空白資料列範本", "輸入範本名稱。", "", MAX_KEY_BYTES)?
            else { return Ok(()) };
            let preferred = name.chars().take(crate::store::MAX_TITLE_CHARS).collect::<String>();
            let Some(title) = crate::dialog::prompt(self.hwnd, "範本預設標題", "輸入每次套用時的預設筆記標題。",
                &preferred, crate::store::MAX_TITLE_CHARS)? else { return Ok(()) };
            let Some(status) = crate::dialog::prompt(self.hwnd, "範本預設狀態", "輸入新資料列的預設狀態。",
                "待處理", MAX_KEY_BYTES)? else { return Ok(()) };
            let Some(body) = self.prompt_template_body("範本正文", "")? else { return Ok(()) };
            self.edit_collection_template_ui(id, None, CollectionTemplate::blank(name, title, status, body))
        }
    }

    unsafe fn capture_collection_template_ui(&self, id: i64) -> Result<()> {
        let rows = self.visible_collection_rows(id)?;
        if rows.is_empty() { return Err("目前檢視沒有可作為範本的資料列。".into()); }
        let hwnd = self.hwnd;
        let anchor = self.widgets.links;
        unsafe {
            let Some(which) = choose(hwnd, anchor, &rows, |(row, status)| format!("{status}　{}  (#{})", row.title, row.id))
            else { return Ok(()); };
            let (row, status) = &rows[which];
            let source = self.notebook.collection_template_source(id, RelationMember { note: row.id, status })?;
            let Some(name) = crate::dialog::prompt(hwnd, "儲存資料列範本", "輸入範本名稱。", "", MAX_KEY_BYTES)?
            else { return Ok(()); };
            let Some(body) = self.prompt_template_body("範本正文", &source.body)? else { return Ok(()) };
            let mut fields = vec![false; source.fields.len()];
            let mut resources = vec![false; source.resources.len()];
            let mut children = Vec::<ChildDraft>::new();
            loop {
                let selection = format!("選擇要帶入的欄位（已選 {}）…", fields.iter().filter(|keep| **keep).count());
                let linked = format!("選擇要帶入的關聯與附件（已選 {}）…", resources.iter().filter(|keep| **keep).count());
                let page_choices = format!("選擇要帶入的直接子頁（已選 {}）…", children.len());
                match popup(hwnd, anchor, None, &[
                    (1,&selection), (2,"選取所有有效型別欄位"), (3,"儲存範本"), (4,"清除欄位選取"), (5,"取消"),
                    (6,&linked), (7,&page_choices),
                ]) {
                    1 => {
                        let choices: Vec<_> = source.fields.iter().zip(&fields).collect();
                        if let Some(index) = choose(hwnd, anchor, &choices, |(field, selected)| {
                            format!("{}　{}　{}", if **selected { "已選" } else { "未選" }, field.key, field.value.trim())
                        }) { fields[index] = !fields[index]; }
                    }
                    2 => fields.fill(true),
                    3 => {
                        let selected: Vec<usize> = fields.iter().enumerate().filter_map(|(index, selected)| selected.then_some(index)).collect();
                        let linked: Vec<_> = resources.iter().enumerate().filter_map(|(index, keep)| keep.then_some(index)).collect();
                        if let Err(error) = self.notebook.capture_collection_template(id, &source, &name, &body, TemplateSelection {
                            fields: &selected, resources: &linked, children: &children,
                        }) {
                            crate::dialog::show_text(hwnd, "無法儲存資料列範本", &error.to_string())?;
                            continue;
                        }
                        label(self.widgets.status, "已儲存資料列範本。");
                        return Ok(());
                    }
                    4 => fields.fill(false),
                    6 => {
                        let choices: Vec<_> = source.resources.iter().zip(&resources).collect();
                        if choices.is_empty() {
                            crate::dialog::show_text(hwnd, "選擇關聯與附件", "這筆資料列目前沒有可選的關聯或附件。")?;
                            continue;
                        }
                        if let Some(index) = choose(hwnd, anchor, &choices, |(item, checked)| {
                            format!("{}　{}", if **checked { "已選" } else { "未選" }, item.label())
                        }) { resources[index] = !resources[index]; }
                    }
                    7 => {
                        let Some(index) = choose(hwnd, anchor, &source.children, |(id, title)| {
                            let checked = if children.iter().any(|child| child.id == *id) { "已選" } else { "未選" };
                            format!("{checked}　{title}  (#{id})")
                        }) else { continue };
                        let id = source.children[index].0;
                        let existing = children.iter().position(|child| child.id == id);
                        if let Some(index) = existing {
                            match popup(hwnd, anchor, None, &[(1,"重新檢閱來源子頁…"),(2,"取消選取這個子頁"),(3,"返回")]) {
                                1 => {}
                                2 => { children.remove(index); continue; }
                                _ => continue,
                            }
                        }
                        let mut child = self.notebook.collection_template_child_source(&source, id)?;
                        let Some(title) = crate::dialog::prompt(hwnd, "範本子頁標題", "檢閱要複製的子頁標題。",
                            &child.content.title, crate::store::MAX_TITLE_CHARS)? else { continue; };
                        let Some(content) = self.prompt_template_body("範本子頁正文", &child.content.body)? else { continue; };
                        let other_bytes: usize = children.iter().filter(|child| child.id != id).map(|child| child.content.body.len()).sum();
                        if body.len()+other_bytes+content.len() > crate::store::MAX_NOTE_BYTES
                            || existing.is_none() && children.len() >= 64
                        {
                            crate::dialog::show_text(hwnd, "無法選取更多子頁", "範本最多 64 個直接子頁，含根頁正文最多 4 MiB。")?;
                            continue;
                        }
                        child.content.title = title;
                        child.content.body = content;
                        if let Some(index) = existing { children[index] = child; } else { children.push(child); }
                    }
                    _ => return Ok(()),
                }
            }
        }
    }

    unsafe fn prompt_template_body(&self, caption: &str, initial: &str) -> Result<Option<String>> {
        let mut body = initial.to_owned();
        unsafe {
            loop {
                let Some(edited) = crate::dialog::prompt_multiline(self.hwnd, caption,
                    "檢閱 Markdown，請移除連結、附件引用、嵌入與 HTML。", &body, crate::store::MAX_NOTE_BYTES)?
                else { return Ok(None) };
                match validate_template_body(&edited) {
                    Ok(()) => return Ok(Some(edited)),
                    Err(error) => crate::dialog::show_text(self.hwnd, "請先調整範本正文", &error.to_string())?,
                }
                body = edited;
            }
        }
    }

    unsafe fn edit_collection_template_ui(
        &self, id: i64, original: Option<&CollectionTemplate>, mut draft: CollectionTemplate,
    ) -> Result<()> {
        let hwnd = self.hwnd;
        let anchor = self.widgets.links;
        let save = if original.is_some() { "儲存範本修改" } else { "儲存新範本" };
        unsafe {
            loop {
                match popup(hwnd, anchor, None, &[
                    (1,"變更範本名稱…"), (2,"變更預設標題…"), (3,"變更預設狀態…"), (4,"變更範本正文…"),
                    (5,"編輯預設欄位值…"), (6,"移除預設欄位…"), (7,save), (8,"取消"),
                    (9,"移除選定的關聯或附件…"), (10,"編輯選定的子頁…"), (11,"新增已宣告型別的預設欄位…"), (12,"新增範本子頁…"),
                ]) {
                    action @ (1 | 2 | 3) => {
                        let (caption, item, limit) = match action {
                            1 => ("變更範本名稱", &mut draft.name, MAX_KEY_BYTES),
                            2 => ("變更預設標題", &mut draft.default_title, crate::store::MAX_TITLE_CHARS),
                            _ => ("變更預設狀態", &mut draft.status, MAX_KEY_BYTES),
                        };
                        if let Some(value) = crate::dialog::prompt(hwnd, caption, "請輸入新值。", item, limit)? { *item = value; }
                    }
                    4 => if let Some(body) = self.prompt_template_body("範本正文", &draft.body)? { draft.body = body; },
                    action @ (5 | 6) => {
                        let Some(index) = choose(hwnd, anchor, &draft.fields, |field| format!("{}　{}", field.key,field.value.trim()))
                        else { continue };
                        if action == 6 {
                            draft.fields.remove(index);
                        } else {
                            let field = &mut draft.fields[index];
                            let value = self.prompt_template_field_value(&field.key, field.kind, &field.value)?;
                            if let Some(value) = value { field.value = value; }
                        }
                    }
                    9 => {
                        if let Some(index) = choose(hwnd, anchor, &draft.resources, |item| item.label()) {
                            draft.resources.remove(index);
                        }
                    }
                    10 => {
                        let Some(index) = choose(hwnd, anchor, &draft.child_paths(), String::clone) else { continue; };
                        match popup(hwnd, anchor, None, &[(1,"變更子頁標題與正文…"), (2,"移除這個子頁"), (3,"返回")]) {
                            1 => {
                                let child = &mut draft.children[index];
                                let Some(title) = crate::dialog::prompt(hwnd, "範本子頁標題", "輸入子頁標題。",
                                    &child.title, crate::store::MAX_TITLE_CHARS)? else { continue; };
                                if let Some(content) = self.prompt_template_body("範本子頁正文", &child.body)? {
                                    child.title = title;
                                    child.body = content;
                                }
                            }
                            2 => {
                                let count = draft.child_removal_count(index);
                                let confirm = format!("移除選定的 {count} 個範本頁面");
                                if popup(hwnd, anchor, None, &[(1,&confirm),(2,"取消")]) == 1 {
                                    draft.remove_child(index);
                                }
                            }
                            _ => {}
                        }
                    }
                    11 => {
                        let available = self.notebook.property_definitions()?.into_iter()
                            .filter(|(key, _)| !draft.fields.iter().any(|field| field.key == *key)).collect::<Vec<_>>();
                        if available.is_empty() {
                            crate::dialog::show_text(hwnd, "新增預設欄位", "沒有其他可選的已宣告型別欄位。")?;
                            continue;
                        }
                        let Some(index) = choose(hwnd, anchor, &available, |(key, kind)| format!("{key}　{}", kind.hint()))
                        else { continue };
                        let (key, kind) = &available[index];
                        if let Some(value) = self.prompt_template_field_value(key, *kind, "")? {
                            draft.fields.push(TemplateField { key: key.clone(), kind: *kind, value });
                        }
                    }
                    12 => {
                        if draft.children.len() >= 64 {
                            crate::dialog::show_text(hwnd, "無法新增範本子頁", "每份範本最多可保存 64 個子頁。")?;
                            continue;
                        }
                        let parents = [(None, "範本根頁".to_owned())].into_iter().chain(
                            draft.child_paths().into_iter().enumerate().map(|(ordinal,path)| (Some(ordinal),path))
                        ).collect::<Vec<_>>();
                        let Some(parent) = choose(hwnd, anchor, &parents, |(_, path)| path.clone())
                        else { continue; };
                        let Some(title) = crate::dialog::prompt(hwnd, "範本子頁標題", "輸入要建立的子頁標題。",
                            "", crate::store::MAX_TITLE_CHARS)? else { continue; };
                        let Some(body) = self.prompt_template_body("範本子頁正文", "")? else { continue; };
                        draft.children.push(TemplateChild { title, body, parent: parents[parent].0 });
                    }
                    7 => {
                        let result = if let Some(current) = original {
                            self.notebook.update_collection_template(id, current, &draft)
                        } else { self.notebook.new_collection_template(id, &draft).map(|_| ()) };
                        if let Err(error) = result {
                            crate::dialog::show_text(hwnd, "無法儲存資料列範本", &error.to_string())?;
                            continue;
                        }
                        label(self.widgets.status, if original.is_some() { "已更新資料列範本。" } else { "已新增資料列範本。" });
                        return Ok(());
                    }
                    _ => return Ok(()),
                }
            }
        }
    }

    unsafe fn prompt_template_field_value(&self, key: &str, kind: PropertyType, initial: &str) -> Result<Option<String>> {
        unsafe {
            if kind == PropertyType::Checkbox {
                Ok(match popup(self.hwnd, self.widgets.links, None, &[(1,"已勾選"),(2,"未勾選")]) {
                    1 => Some("true".to_owned()), 2 => Some("false".to_owned()), _ => None,
                })
            } else {
                crate::dialog::prompt(self.hwnd, key, kind.hint(), initial, MAX_VALUE_BYTES)
            }
        }
    }

    pub(super) unsafe fn edit_collection_row_field(&mut self, id: i64, title: String) -> Result<()> {
        let fields = self.notebook.property_definitions()?;
        if fields.is_empty() { return Err("請先在工作區設定數字、核取方塊或日期型別。".into()); }
        let rows = self.visible_collection_rows(id)?;
        if rows.is_empty() { return Err("目前檢視沒有可編輯的資料列。".into()); }
        let hwnd = self.hwnd;
        let anchor = self.widgets.links;
        let Some(index) = (unsafe { choose(hwnd, anchor, &rows, |(row, status)| format!("{status}　{}  ({})", row.title, row.id)) })
        else { return Ok(()) };
        let (row, status) = &rows[index];
        let props = self.notebook.properties(row.id)?;
        let raw = |key: &str| props.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str());
        let Some(index) = (unsafe { choose(hwnd, anchor, &fields, |(key, kind)| {
            let preview = raw(key).map(|value| value.lines().next().unwrap_or("").chars().take(42).collect::<String>())
                .unwrap_or_else(|| "未設定".into());
            format!("{key}　{preview}　{}", kind.hint())
        }) }) else { return Ok(()) };
        let (key, kind) = &fields[index];
        let previous = raw(key);
        let mut draft = previous.unwrap_or_default().to_owned();
        loop {
            let next = if *kind == PropertyType::Checkbox {
                match unsafe { popup(hwnd, anchor, None, &[(1,"勾選"),(2,"取消勾選"),(3,"清除欄位"),(4,"取消編輯")]) } {
                    1 => Some("true".to_owned()), 2 => Some("false".to_owned()), 3 => None,
                    _ => return Ok(()),
                }
            } else {
                let hint = format!("{}。留白會清除這一欄。", kind.hint());
                let Some(value) = (unsafe { crate::dialog::prompt(hwnd,key,&hint,&draft,MAX_VALUE_BYTES)? })
                else { return Ok(()); };
                draft = value;
                (!draft.trim().is_empty()).then(|| draft.clone())
            };
            match self.notebook.edit_collection_field(id, row.id, status,
                FieldEdit { key, kind: *kind, previous, value: next.as_deref() })
            {
                Ok(()) => break,
                Err(error) => unsafe {
                    let text = if *kind == PropertyType::Checkbox {
                        next.as_deref().unwrap_or("（清除欄位）")
                    } else { &draft };
                    crate::dialog::show_text(hwnd,"欄位未儲存",&format!("{error}\r\n\r\n{text}"))?;
                },
            }
        }
        unsafe {
            if let Some(view) = self.active_collection_view(id) {
                self.reopen_collection_view(id, title, view.id)?;
            } else {
                self.open_collection_layout(id, title, None)?;
            }
            label(self.widgets.status, "已更新資料列欄位。");
        }
        Ok(())
    }

    pub(super) unsafe fn edit_collection_row_relations(&mut self, id: i64, title: String) -> Result<()> {
        let rows = self.visible_collection_rows(id)?;
        if rows.is_empty() { return Err("目前檢視沒有可編輯的資料列。".into()); }
        let hwnd = self.hwnd;
        let anchor = self.widgets.links;
        let Some(index) = (unsafe { choose(hwnd, anchor, &rows, |(row, status)| format!("{status}　{}  (#{})", row.title, row.id)) })
        else { return Ok(()); };
        let (row, status) = &rows[index];
        let source = RelationMember { note: row.id, status };
        let action = unsafe { popup(hwnd, anchor, None, &[
            (1, "新增同集合關聯…"), (2, "開啟同集合關聯目標…"),
            (3, "移除同集合關聯…"), (4, "查看資料列的關聯彙總…"),
            (5, "新增跨集合關聯…"), (6, "開啟關聯目標（含集合外）…"),
            (7, "移除關聯（含集合外）…"),
        ]) };
        unsafe {
            match action {
                1 | 5 => {
                    let (target_collection, target_label) = if action == 5 {
                        let other: Vec<_> = self.notebook.collections()?.into_iter()
                            .filter(|(collection, _)| *collection != id).collect();
                        if other.is_empty() { return Err("目前沒有其他集合可供選取。".into()); }
                        let Some(index) = choose(hwnd, anchor, &other, |(id, title)| format!("{title}  (#{id})"))
                        else { return Ok(()); };
                        (other[index].0, other[index].1.clone())
                    } else { (id, "同集合".to_owned()) };
                    let members: Vec<_> = self.notebook.collection_rows(target_collection)?.into_iter()
                        .filter(|(row, _)| row.id != source.note).collect();
                    if members.is_empty() { return Err("選取的集合沒有其他可用的目標資料列。".into()); }
                    let Some(index) = choose(hwnd, anchor, &members, |(row, status)| {
                        format!("{target_label}　{status}　{}  (#{})", row.title, row.id)
                    }) else { return Ok(()); };
                    let (row, status) = &members[index];
                    let target = RelationMember { note: row.id, status };
                    let prompt = format!("從來源 #{} 指向「{target_label}」的筆記 #{}。輸入關聯名稱。", source.note, target.note);
                    let Some(name) = crate::dialog::prompt(hwnd, "新增集合關聯", &prompt, "", MAX_KEY_BYTES)?
                    else { return Ok(()); };
                    self.notebook.add_collection_relation_between(id, source, &name, target_collection, target)?;
                }
                2 | 3 => {
                    let links = self.notebook.collection_relations(id, source)?;
                    if links.is_empty() { return Err("此列沒有指向同集合有效資料列的關聯。".into()); }
                    let Some(index) = choose(hwnd, anchor, &links, |(name, row, status)| {
                        format!("{name}　{status}　{}  (#{})", row.title, row.id)
                    }) else { return Ok(()); };
                    let (name, row, status) = &links[index];
                    if action == 2 {
                        if !self.notebook.collection_relations(id, source)?.iter()
                            .any(|(current_name, current_row, current_status)|
                                current_name == name && current_row.id == row.id && current_status == status)
                        {
                            return Err("關聯或目標狀態已變更，請重新整理。".into());
                        }
                        self.open_import(row.id, "已開啟集合關聯目標。");
                        return Ok(());
                    }
                    let confirm = format!("移除「{name}」指向 #{} 的關聯", row.id);
                    if popup(hwnd, anchor, None, &[(1, &confirm), (2, "取消")]) != 1 { return Ok(()); }
                    let target = RelationMember { note: row.id, status };
                    self.notebook.set_collection_relation(id, source, name, target, false)?;
                }
                4 => return self.show_collection_relation_rollup(id, source),
                6 | 7 => {
                    let links = self.notebook.inspect_collection_links(id, source)?;
                    if links.is_empty() { return Err("此列沒有具名關聯。".into()); }
                    let Some(index) = choose(hwnd, anchor, &links, |link| {
                        let scope = if link.trash { "垃圾桶".into() }
                            else if let Some(status) = &link.current_status { format!("同集合／{status}") }
                            else { "目前集合之外".into() };
                        format!("{}　{scope}　{}  (#{})", link.name, link.title, link.target)
                    }) else { return Ok(()); };
                    let link = &links[index];
                    if action == 6 {
                        let target = self.notebook.open_inspected_collection_link(id, source, link)?;
                        self.open_import(target, "已開啟集合關聯目標。");
                        return Ok(());
                    }
                    let confirm = format!("移除「{}」指向 #{} 的關聯", link.name, link.target);
                    if popup(hwnd, anchor, None, &[(1, &confirm), (2, "取消")]) != 1 { return Ok(()); }
                    self.notebook.remove_inspected_collection_link(id, source, link)?;
                }
                _ => return Ok(()),
            }
            self.open_kanban(id, title)?;
            label(self.widgets.status, if matches!(action, 1 | 5) { "已新增關聯。公式與彙總會讀取最新值。" }
                else { "已移除關聯。公式與彙總會讀取最新值。" });
        }
        Ok(())
    }

    unsafe fn show_collection_relation_rollup(&self, id: i64, source: RelationMember<'_>) -> Result<()> {
        let hwnd = self.hwnd;
        let Some(name) = (unsafe { crate::dialog::prompt(hwnd, "資料列關聯彙總", "關聯名稱", "", MAX_KEY_BYTES)? })
        else { return Ok(()); };
        let Some(property) = (unsafe {
            crate::dialog::prompt(hwnd, "資料列關聯彙總", "數值欄位名稱（留白僅查看筆數）", "", MAX_KEY_BYTES)?
        }) else { return Ok(()); };
        let property = property.trim();
        let (in_collection, total, sum) = self.notebook.collection_relation_totals(
            id, source, &name, (!property.is_empty()).then_some(property),
        )?;
        let mut result = format!("{}\r\n同集合有效目標　{in_collection}\r\n所有有效目標（含集合外）　{total}", name.trim());
        if let Some(sum) = sum {
            write!(result, "\r\n所有有效目標的「{property}」總和　{sum}")?;
        }
        unsafe { crate::dialog::show_text(hwnd, "資料列關聯彙總", &result) }
    }

    pub(super) unsafe fn collection_checkbox_menu(&mut self, id: i64, title: String) -> Result<()> {
        let definitions = self.notebook.property_definitions()?.into_iter()
            .filter(|(_, kind)| *kind == PropertyType::Checkbox)
            .map(|(key, _)| key)
            .collect::<Vec<_>>();
        if definitions.is_empty() { return Err("請先在工作區設定核取方塊屬性。".into()); }
        let Some(which) = (unsafe { choose(self.hwnd,self.widgets.links,&definitions,String::clone) })
        else { return Ok(()) };
        let key = &definitions[which];
        let rows = self.visible_collection_rows(id)?;
        let values = self.notebook.collection_property_values(id,key)?;
        let Some(which) = (unsafe { choose(self.hwnd,self.widgets.links,&rows,|(row,status)| {
            let state = checkbox_label(values.get(&row.id).map(String::as_str));
            format!("{state}　{status}　{}  ({})",row.title,row.id)
        }) }) else { return Ok(()) };
        let (row,status) = &rows[which];
        let checked = self.notebook.toggle_collection_checkbox(id,row.id,status,key,values.get(&row.id).map(String::as_str))?;
        let saved = self.active_collection_view(id);
        unsafe {
            self.open_collection_layout(id,title,saved)?;
            label(self.widgets.status, if checked { "已勾選" } else { "已取消勾選" });
        }
        Ok(())
    }

    unsafe fn reopen_collection_view(&mut self, id: i64, title: String, view_id: i64) -> Result<()> {
        let view = self.notebook.collection_views(id)?.into_iter().find(|view| view.id == view_id)
            .ok_or("這個檢視已被刪除。")?;
        unsafe { self.open_collection_layout(id, title, Some(view)) }
    }

    unsafe fn edit_collection_rules(&mut self, id: i64, title: String) -> Result<()> {
        let view = self.active_collection_view(id).ok_or("請先儲存或開啟一個集合檢視。")?;
        let mut rules = view.rules();
        let hwnd = self.hwnd;
        let anchor = self.widgets.links;
        let fields = self.notebook.property_definitions()?;
        unsafe {
            loop {
                let group_room = FilterRoom::FULL.without((rules.filters.len(), 0));
                let leaf_limit = FilterRoom::FULL.without(FilterGroup::counts_in(&rules.groups)).leaves;
                let summary = format!("檢視或移除條件與排序（{} 個篩選，{} 欄排序）…", rules.filters.len(), rules.sorts.len());
                match popup(hwnd, anchor, None, &[
                    (1,"新增篩選…"), (2,"新增排序…"), (3,filter_toggle_label(rules.mode)), (4,&summary), (5,"儲存變更"),
                    (6,"取消所有未儲存變更"), (7,"新增篩選群組…"), (8,"編輯篩選群組…"),
                    (9,"查看衍生欄位的篩選或排序錯誤…"),
                ]) {
                    1 => {
                        if rules.filters.len() == leaf_limit {
                            crate::dialog::show_text(hwnd, "無法新增篩選", FILTER_LIMIT)?;
                        } else if let Some(rule) = self.prompt_collection_filter(id, &fields)? {
                            rules.filters.push(rule);
                        }
                    }
                    2 => {
                        let Some((key, kind, argument)) = self.pick_collection_column(id, &fields, true)?
                        else { continue };
                        let source = match kind {
                            FilterKind::Typed(kind) => SortSource::Property(Some(kind)),
                            FilterKind::Text => SortSource::Property(None),
                            FilterKind::Formula(kind) => SortSource::Formula(kind),
                            FilterKind::RollupCount => SortSource::RollupCount,
                            FilterKind::RollupSum => SortSource::RollupSum(argument.ok_or("加總缺少數值欄位。")?),
                            _ => return Err("這個欄位無法用於排序。".into()),
                        };
                        let descending = match popup(hwnd, anchor, None, &[(1,"升冪"),(2,"降冪")]) {
                            1 => false, 2 => true, _ => continue,
                        };
                        let rule = SortRule { key, source, descending };
                        if let Some(sort) = rules.sorts.iter_mut().find(|sort| sort.same_column(&rule)) {
                            *sort = rule;
                        } else {
                            rules.sorts.push(rule);
                        }
                    }
                    3 => toggle_filter_mode(&mut rules.mode),
                    4 => {
                        let mut entries = rules.filters.iter().map(filter_summary).collect::<Vec<_>>();
                        entries.extend(rules.sorts.iter().enumerate().map(|(index, sort)|
                            format!("第 {} 排序  {}  {}", index + 1, sort.label(), if sort.descending {"降冪"} else {"升冪"})
                        ));
                        let Some(index) = choose(hwnd, anchor, &entries, String::clone) else { continue };
                        if index < rules.filters.len() {
                            self.edit_or_remove_filter(id, &mut rules.filters, index, &fields, leaf_limit)?;
                        } else {
                            let index = index - rules.filters.len();
                            match popup(hwnd, anchor, None, &[(1,"移除這項排序"),(2,"提高優先順序"),(3,"降低優先順序"),(4,"返回")]) {
                                1 => { rules.sorts.remove(index); }
                                2 if index > 0 => rules.sorts.swap(index, index - 1),
                                3 if index + 1 < rules.sorts.len() => rules.sorts.swap(index, index + 1),
                                _ => {}
                            }
                        }
                    }
                    5 => {
                        if let Err(error) = self.notebook.set_collection_view_rules(id, &view, rules.clone()) {
                            crate::dialog::show_text(hwnd, "無法儲存檢視", &error.to_string())?;
                            continue;
                        }
                        return self.reopen_collection_view(id, title, view.id);
                    }
                    7 => self.add_filter_group(id, &mut rules.groups, &fields, 1, group_room)?,
                    8 => self.pick_filter_group(id, &mut rules.groups, &fields, 1, group_room)?,
                    9 => {
                        let problems = self.notebook.collection_rule_problems(id, &view)?;
                        let mut report = if problems.entries.is_empty() {
                            "已儲存的規則沒有遇到衍生欄位錯誤。".to_owned()
                        } else {
                            format!("已儲存規則中實際評估到的錯誤\r\n\r\n{}", problems.entries.join("\r\n"))
                        };
                        if problems.omitted > 0 { write!(report, "\r\n另有 {} 項。", problems.omitted)?; }
                        crate::dialog::show_text(hwnd, "衍生欄位的篩選與排序錯誤", &report)?;
                    }
                    _ => return Ok(()),
                }
            }
        }
    }

    unsafe fn pick_collection_column(
        &self, collection: i64, fields: &[(String, PropertyType)], sorting: bool,
    ) -> Result<Option<(String, FilterKind, Option<String>)>> {
        let hwnd = self.hwnd;
        let anchor = self.widgets.links;
        let mut definitions: Vec<_> = fields.iter().map(|(key, kind)| {
            (key.as_str(), FilterKind::Typed(*kind))
        }).collect();
        if !sorting { definitions.extend([("", FilterKind::Title), ("", FilterKind::Status)]); }
        definitions.extend([("", FilterKind::Text), ("", FilterKind::Formula(None))]);
        if !sorting { definitions.push(("", FilterKind::Relation)); }
        definitions.extend([("", FilterKind::RollupCount), ("", FilterKind::RollupSum)]);
        unsafe {
            let Some(index) = choose(hwnd, anchor, &definitions, |(key, kind)| match kind {
                FilterKind::Title => "筆記標題".into(),
                FilterKind::Status => "集合狀態".into(),
                FilterKind::Text => "其他文字欄位…".into(),
                FilterKind::Formula(_) => "公式結果…".into(),
                FilterKind::Relation => "關聯目標…".into(),
                FilterKind::RollupCount => "彙總筆數…".into(),
                FilterKind::RollupSum => "彙總加總…".into(),
                _ => format!("{key}　{}", kind.hint()),
            }) else { return Ok(None) };
            let (column, mut kind) = definitions[index];
            let mut argument = None;
            let key = match kind {
                FilterKind::Text => {
                    let Some(value) = crate::dialog::prompt(hwnd, "文字欄位", "輸入未宣告型別的文字欄位名稱。", "", MAX_KEY_BYTES)?
                    else { return Ok(None) };
                    value
                }
                FilterKind::Formula(_) => {
                    let formulas = self.notebook.collection_formulas(collection)?;
                    if formulas.is_empty() { return Err("請先在這個集合儲存一個公式。".into()); }
                    let Some(which) = choose(hwnd, anchor, &formulas, |(name, _)| name.clone()) else { return Ok(None); };
                    kind = match popup(hwnd, anchor, None, &[
                        (1,"數字結果"), (2,"核取方塊結果"), (3,"日期結果"), (4,"文字結果"),
                    ]) {
                        1 => FilterKind::Formula(Some(PropertyType::Number)),
                        2 => FilterKind::Formula(Some(PropertyType::Checkbox)),
                        3 => FilterKind::Formula(Some(PropertyType::Date)),
                        4 => FilterKind::Formula(None),
                        _ => return Ok(None),
                    };
                    formulas[which].0.clone()
                }
                FilterKind::Relation | FilterKind::RollupCount | FilterKind::RollupSum => {
                    let Some(name) = crate::dialog::prompt(hwnd, "關聯或彙總欄位", "輸入具名關聯的名稱。", "", MAX_KEY_BYTES)?
                    else { return Ok(None) };
                    if kind == FilterKind::RollupSum {
                        let Some(field) = crate::dialog::prompt(hwnd, "彙總加總欄位", "輸入關聯目標的數值欄位名稱。", "", MAX_KEY_BYTES)?
                        else { return Ok(None) };
                        argument = Some(field);
                    }
                    name
                }
                _ => column.to_owned(),
            };
            Ok(Some((key, kind, argument)))
        }
    }

    unsafe fn prompt_collection_filter(&self, collection: i64, fields: &[(String, PropertyType)]) -> Result<Option<FilterRule>> {
        let hwnd = self.hwnd;
        let anchor = self.widgets.links;
        unsafe {
            let Some((key, kind, argument)) = self.pick_collection_column(collection, fields, false)? else { return Ok(None); };
            let choices: Vec<_> = FilterOperator::OPTIONS.into_iter().enumerate()
                .filter(|(_, (op, _))| kind.allows(*op)).map(|(index, (_, label))| (index + 1, label)).collect();
            let Some(index) = popup(hwnd, anchor, None, &choices).checked_sub(1) else { return Ok(None) };
            let Some((operator, caption)) = FilterOperator::OPTIONS.get(index).copied() else { return Ok(None) };
            let value = if !operator.needs_value() { String::new() }
                else if kind == FilterKind::Relation {
                    let notes = self.notebook.rows(View::All, "")?;
                    let Some(which) = choose(hwnd, anchor, &notes, |row| format!("{}  (#{})", row.title, row.id))
                    else { return Ok(None); };
                    notes[which].id.to_string()
                } else if kind.checkbox() {
                    match popup(hwnd, anchor, None, &[(1,"已勾選"),(2,"未勾選")]) {
                        1 => "true".to_owned(), 2 => "false".to_owned(), _ => return Ok(None),
                    }
                } else {
                    let hint = if operator.day_window() { "輸入 1 到 365 日。" } else { kind.hint() };
                    let Some(value) = crate::dialog::prompt(hwnd, caption, hint, "", MAX_VALUE_BYTES)?
                    else { return Ok(None) };
                    value
                };
            Ok(Some(FilterRule { key, kind, operator, value, argument }))
        }
    }

    unsafe fn edit_or_remove_filter(&self, collection: i64, filters: &mut Vec<FilterRule>, index: usize, fields: &[(String, PropertyType)], leaf_limit: usize) -> Result<()> {
        unsafe {
            let mut actions = vec![(1,"移除這項篩選"),(2,"返回"),(3,"重設這項篩選…"),(4,"複製這項篩選")];
            if index > 0 { actions.push((5,"將條件往前移")); }
            if index + 1 < filters.len() { actions.push((6,"將條件往後移")); }
            match popup(self.hwnd, self.widgets.links, None, &actions) {
                1 => { filters.remove(index); }
                3 => {
                    if let Some(rule) = self.prompt_collection_filter(collection, fields)? { filters[index] = rule; }
                }
                4 if filters.len() == leaf_limit => crate::dialog::show_text(self.hwnd, "無法複製篩選", FILTER_LIMIT)?,
                4 => filters.insert(index + 1, filters[index].clone()),
                5 if index > 0 => filters.swap(index, index - 1),
                6 if index + 1 < filters.len() => filters.swap(index, index + 1),
                _ => {}
            }
        }
        Ok(())
    }

    unsafe fn add_filter_group(&self, collection: i64, groups: &mut Vec<FilterGroup>, fields: &[(String, PropertyType)], depth: usize, room: FilterRoom) -> Result<()> {
        let free = room.without(FilterGroup::counts_in(groups));
        if !free.holds((1, 1)) {
            return unsafe { crate::dialog::show_text(self.hwnd, "無法新增篩選群組", FILTER_LIMIT) };
        }
        let mut group = FilterGroup::default();
        if unsafe { self.edit_filter_group(collection, &mut group, fields, depth, free)? } {
            groups.push(group);
        }
        Ok(())
    }

    unsafe fn pick_filter_group(&self, collection: i64, groups: &mut Vec<FilterGroup>, fields: &[(String, PropertyType)], depth: usize, room: FilterRoom) -> Result<()> {
        let indexed: Vec<_> = groups.iter().enumerate().collect();
        unsafe {
            let Some(index) = choose(self.hwnd, self.widgets.links, &indexed, |(i, group)| {
                let mode = if group.mode == FilterMode::All { "全部相符" } else { "任一相符" };
                format!("群組 {}　{mode}　{} 條件、{} 子群組", i + 1, group.filters.len(), group.groups.len())
            }) else { return Ok(()) };
            let mut actions = vec![(1,"編輯此群組…"),(2,"移除此群組"),(3,"返回"),(4,"複製此群組")];
            if index > 0 { actions.push((5,"將群組往前移")); }
            if index + 1 < groups.len() { actions.push((6,"將群組往後移")); }
            let (leaves, group_count) = FilterGroup::counts_in(groups);
            let (selected_leaves, selected_groups) = groups[index].counts();
            match popup(self.hwnd, self.widgets.links, None, &actions) {
                1 => {
                    let edit_room = room.without((leaves - selected_leaves, group_count - selected_groups));
                    let mut draft = groups[index].clone();
                    if self.edit_filter_group(collection, &mut draft, fields, depth, edit_room)? { groups[index] = draft; }
                }
                2 => { groups.remove(index); }
                4 if !room.without((leaves, group_count)).holds((selected_leaves, selected_groups)) => {
                    crate::dialog::show_text(self.hwnd, "無法複製篩選群組", FILTER_LIMIT)?;
                }
                4 => groups.insert(index + 1, groups[index].clone()),
                5 if index > 0 => groups.swap(index, index - 1),
                6 if index + 1 < groups.len() => groups.swap(index, index + 1),
                _ => {}
            }
        }
        Ok(())
    }

    unsafe fn edit_filter_group(&self, collection: i64, group: &mut FilterGroup, fields: &[(String, PropertyType)], depth: usize, room: FilterRoom) -> Result<bool> {
        let hwnd = self.hwnd;
        let anchor = self.widgets.links;
        unsafe {
            loop {
                let child_room = room.without((group.filters.len(), 1));
                let leaf_limit = room.without(FilterGroup::counts_in(&group.groups)).leaves;
                let condition_list = format!("編輯群組內條件（{} 個）…", group.filters.len());
                let child_groups = format!("編輯下一層群組（{} 個）…", group.groups.len());
                let mut actions = vec![(1,"新增群組內條件…"), (3,filter_toggle_label(group.mode)), (4,condition_list.as_str()),
                    (5,child_groups.as_str()), (6,"套用群組並返回"), (7,"放棄這次群組變更")];
                if depth < MAX_FILTER_GROUP_DEPTH { actions.insert(1, (2,"新增下一層群組…")); }
                match popup(hwnd, anchor, None, &actions) {
                    1 => {
                        if group.filters.len() == leaf_limit {
                            crate::dialog::show_text(hwnd, "無法新增篩選", FILTER_LIMIT)?;
                        } else if let Some(rule) = self.prompt_collection_filter(collection, fields)? {
                            group.filters.push(rule);
                        }
                    }
                    2 if depth < MAX_FILTER_GROUP_DEPTH => self.add_filter_group(collection, &mut group.groups, fields, depth + 1, child_room)?,
                    3 => toggle_filter_mode(&mut group.mode),
                    4 => {
                        if let Some(index) = choose(hwnd, anchor, &group.filters, filter_summary) {
                            self.edit_or_remove_filter(collection, &mut group.filters, index, fields, leaf_limit)?;
                        }
                    }
                    5 => self.pick_filter_group(collection, &mut group.groups, fields, depth + 1, child_room)?,
                    6 if group.is_empty() => {
                        crate::dialog::show_text(hwnd, "無法套用群組", "請至少留下一個篩選條件或子群組。")?;
                    }
                    6 => return Ok(true),
                    _ => return Ok(false),
                }
            }
        }
    }

    unsafe fn edit_collection_end_key(&mut self, id: i64, title: String) -> Result<()> {
        let view = self.active_collection_view(id).ok_or("請先開啟時間軸、月曆或週曆。")?;
        let start = view.presentation.calendar_key.as_deref().ok_or("請先開啟時間軸、月曆或週曆。")?;
        let mut fields = vec![None];
        fields.extend(self.notebook.property_definitions()?.into_iter()
            .filter(|(key, kind)| key != start && *kind == PropertyType::Date).map(|(key, _)| Some(key)));
        let Some(index) = (unsafe { choose(self.hwnd, self.widgets.links, &fields, |field| {
            let label = field.as_deref().unwrap_or("僅使用起始日");
            if *field == view.presentation.calendar_end_key { format!("目前使用　{label}") } else { label.to_owned() }
        }) }) else { return Ok(()) };
        self.notebook.set_collection_end_key(id, &view, fields[index].as_deref())?;
        unsafe { self.reopen_collection_view(id, title, view.id) }
    }

    pub(super) unsafe fn edit_collection_row_dates(&mut self, id: i64, title: String) -> Result<()> {
        let view = self.active_collection_view(id).ok_or("請先開啟時間軸、月曆或週曆。")?;
        let start = view.presentation.calendar_key.as_deref().ok_or("請先開啟時間軸、月曆或週曆。")?;
        let end = view.presentation.calendar_end_key.as_deref();
        let rows = self.notebook.collection_rows(id)?;
        let Some(index) = (unsafe { choose(self.hwnd, self.widgets.links, &rows, |(row, status)| format!("{status}　{}  ({})", row.title, row.id)) })
        else { return Ok(()) };
        let note = rows[index].0.id;
        let props = self.notebook.properties(note)?;
        let [was_start, was_end] = [Some(start), end].map(|key|
            key.and_then(|key| props.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str()))
        );
        let input = |key: &str, old: Option<&str>, can_clear: &str| unsafe {
            let old = old.unwrap_or_default();
            crate::dialog::prompt(self.hwnd, key, can_clear, old, old.chars().count().max(10))
        };
        let Some(first) = input(start, was_start, "起始日 YYYY-MM-DD，留白清除")? else { return Ok(()) };
        let last = if let Some(end) = end {
            let Some(last) = input(end, was_end, "結束日 YYYY-MM-DD（含當日），留白為單日")? else { return Ok(()) };
            last
        } else { String::new() };
        let save = || self.notebook.set_collection_row_dates(id, &view, note,
            [was_start, was_end], [parse_day(&first)?, parse_day(&last)?]);
        if let Err(error) = save() {
            return unsafe { crate::dialog::show_text(self.hwnd,"日期欄位未儲存（可複製）",
                &format!("{error}\r\n\r\n起始日：{first}\r\n結束日：{last}")) };
        }
        unsafe { self.reopen_collection_view(id, title, view.id) }
    }

    unsafe fn edit_collection_range(&mut self,id:i64,title:String) -> Result<()> {
        let Some(mut saved) = self.active_collection_view(id).filter(|view|view.presentation.calendar_key.is_some())
        else { return Err("請先開啟時間軸、月曆或週曆。".into()); };
        let previous_start = saved.presentation.calendar_from.map(CalendarDay::iso).unwrap_or_default();
        let previous_end = saved.presentation.calendar_until.map(CalendarDay::iso).unwrap_or_default();
        let Some(start) = (unsafe { crate::dialog::prompt(self.hwnd,"日期範圍","起始日期 YYYY-MM-DD（含當日，留空不限）",&previous_start,10)? })
        else { return Ok(()) };
        let Some(end) = (unsafe { crate::dialog::prompt(self.hwnd,"日期範圍","結束日期 YYYY-MM-DD（含當日，留空不限）",&previous_end,10)? })
        else { return Ok(()) };
        let (start,end) = (parse_day(&start)?,parse_day(&end)?);
        self.notebook.set_collection_calendar_range(id,&saved,start,end)?;
        saved.presentation.calendar_from = start;
        saved.presentation.calendar_until = end;
        unsafe { self.open_collection_layout(id,title,Some(saved)) }
    }

    pub(super) unsafe fn shift_collection_month(&mut self,id:i64,title:String,delta:i32) -> Result<()> {
        let Some(WorkspaceView::Kanban(view)) = &self.extras.workspace else { return Ok(()) };
        let Some(mut saved) = view.saved_view.clone() else { return Ok(()) };
        if let Some((week,_)) = &view.week {
            let Some(week) = week.shifted(if delta<0 {-7} else {7}) else { return Ok(()) };
            self.notebook.set_collection_calendar_week(id,&saved,week)?;
            saved.presentation.calendar_week_day = Some(week);
            return unsafe { self.open_collection_layout(id,title,Some(saved)) };
        }
        let month = view.timeline.as_ref().map(|(month, _)| *month)
            .or_else(|| view.calendar.as_ref().map(|(month, _)| *month));
        let Some(month) = month else { return Ok(()) };
        let month = month.shifted(delta);
        self.notebook.set_collection_calendar_month(id,&saved,month)?;
        saved.presentation.calendar_month = Some(month);
        unsafe { self.open_collection_layout(id,title,Some(saved)) }
    }

    pub(super) unsafe fn open_collection_date(&mut self, day:Option<usize>) -> Result<()> {
        let Some(WorkspaceView::Kanban(view)) = &self.extras.workspace else { return Ok(()) };
        if let Some((_, rows)) = &view.timeline {
            let Some(index) = (unsafe { choose(self.hwnd, self.widgets.links, rows, |event| {
                format!("{} 至 {}　{}　{}  (#{})", event.start.iso(), event.end.iso(), event.status, event.row.title, event.row.id)
            }) }) else { return Ok(()) };
            let note = rows[index].row.id;
            unsafe { self.open_import(note, "已開啟時間軸筆記。"); }
            return Ok(());
        }
        if view.week.is_some() { return unsafe { self.open_collection_week_day(None) }; }
        let Some((month,days)) = &view.calendar else { return Ok(()) };
        let rows = days.iter().filter(|(number,_)| day.is_none_or(|d|d==**number))
            .flat_map(|(&number,rows)|rows.iter().map(move |row|(number,row))).collect::<Vec<_>>();
        if rows.is_empty() { return Ok(()); }
        let chosen = if day.is_some() && rows.len()==1 { Some(0) } else {
            unsafe { choose(self.hwnd,self.widgets.links,&rows,|(day,row)|
                format!("{:04}-{:02}-{day:02}　{}",month.year,month.month,row.title)) }
        };
        if let Some(index) = chosen {
            let note = rows[index].1.id;
            unsafe { self.open_import(note,"已開啟月曆筆記"); }
        }
        Ok(())
    }

    unsafe fn open_collection_week_day(&mut self, day:Option<CalendarDay>) -> Result<()> {
        let Some(WorkspaceView::Kanban(view)) = &self.extras.workspace else { return Ok(()) };
        let Some((_,days)) = &view.week else { return Ok(()) };
        let rows = days.iter().filter(|(date,_)|day.is_none_or(|wanted|wanted==**date))
            .flat_map(|(date,rows)|rows.iter().map(move |row|(*date,row))).collect::<Vec<_>>();
        if rows.is_empty() { return Ok(()); }
        let chosen = if day.is_some() && rows.len()==1 { Some(0) } else {
            unsafe { choose(self.hwnd,self.widgets.links,&rows,|(date,row)|format!("{}　{}",date.iso(),row.title)) }
        };
        if let Some(index) = chosen {
            let note = rows[index].1.id;
            unsafe { self.open_import(note,"已開啟月曆筆記"); }
        }
        Ok(())
    }

    pub(super) unsafe fn click_collection_view(&mut self, lp: LPARAM) -> Result<()> {
        let (x,y) = (lp as u16 as i16 as i32,(lp >> 16) as u16 as i16 as i32);
        let area = unsafe { self.board_area() };
        if unsafe { self.click_collection_fold(area, x, y)? } { return Ok(()); }
        if let Some(WorkspaceView::Kanban(view)) = &self.extras.workspace {
            if let Some((week,_)) = &view.week {
                let grid = self.calendar_grid(area);
                if grid.right-grid.left>=7 && x>=grid.left && x<grid.right && y>=grid.top && y<grid.bottom {
                    let column = (7*(x-grid.left+1)-1)/(grid.right-grid.left);
                    if let Some(day) = week.shifted(column as i8) {
                        return unsafe { self.open_collection_week_day(Some(day)) };
                    }
                }
                return Ok(());
            }
            if view.is_list() {
                let (left,right,top,step) = self.collection_table_rect(area);
                let top = view.first_table_line(top, step);
                if x>=left && x<right && y>=top && y<top+((area.bottom-top)/step)*step {
                    let offset = ((y-top)/step) as usize;
                    let note = match view.table_lines().nth(view.row+offset) {
                        Some(CollectionLine::Note(_, note)) => Some(note.id),
                        _ => None,
                    };
                    if let Some(note) = note { unsafe { self.open_import(note,"已開啟清單筆記"); } }
                }
                return Ok(());
            }
        }
        if let Some(WorkspaceView::Kanban(view)) = &self.extras.workspace
            && let Some((month,_)) = &view.calendar
        {
            let area = unsafe { self.board_area() };
            let grid = self.calendar_grid(area);
            let (x,y) = (lp as u16 as i16 as i32,(lp >> 16) as u16 as i16 as i32);
            if x >= grid.left && x < grid.right && y >= grid.top && y < grid.bottom {
                let column = (7*(x-grid.left+1)-1)/(grid.right-grid.left);
                let week = (6*(y-grid.top+1)-1)/(grid.bottom-grid.top);
                let slot = (week*7+column) as usize;
                if let Some(day) = (slot+1).checked_sub(month.sunday_offset()).filter(|day| *day>0 && *day<=month.days()) {
                    return unsafe { self.open_collection_date(Some(day)) };
                }
            }
            return Ok(());
        }
        unsafe { self.click_collection_checkbox(lp) }
    }

    pub(super) unsafe fn cancel_timeline_drag(&mut self) -> bool {
        let Some(WorkspaceView::Kanban(view)) = self.extras.workspace.as_mut() else { return false };
        if view.timeline_drag.take().is_none() { return false; }
        unsafe {
            if GetCapture() == self.hwnd { ReleaseCapture(); }
            InvalidateRect(self.hwnd, &self.board_area(), 0);
        }
        true
    }

    pub(super) unsafe fn timeline_input(&mut self, msg: u32, lp: LPARAM) -> Result<bool> {
        if matches!(msg, WM_CAPTURECHANGED | WM_CANCELMODE) {
            return Ok(unsafe { self.cancel_timeline_drag() });
        }
        let point = [lp as u16 as i16 as i32, (lp >> 16) as u16 as i16 as i32];
        let area = unsafe { self.board_area() };
        let Some(WorkspaceView::Kanban(view)) = &self.extras.workspace else { return Ok(false) };
        let Some((month, entries)) = &view.timeline else { return Ok(false) };
        let month = *month;
        let grid = self.timeline_grid(area, month);
        match msg {
            WM_LBUTTONDOWN if view.timeline_drag.is_none() => {
                let Some(grid) = grid else { return Ok(true) };
                let Some(row) = grid.row_at(point[1]).and_then(|row| entries.get(view.row + row)) else { return Ok(true) };
                let left = grid.day_edge(row.visible_start.day() - 1, month.days());
                let right = grid.day_edge(row.visible_end.day(), month.days());
                let on_bar = (left..right).contains(&point[0]);
                if !on_bar && !(grid.label_left..grid.chart.left).contains(&point[0]) { return Ok(true); }
                let Some(saved) = &view.saved_view else { return Ok(true) };
                let properties = self.notebook.properties(row.row.id)?;
                let keys = [saved.presentation.calendar_key.as_deref(), saved.presentation.calendar_end_key.as_deref()];
                let expected = keys.map(|key| key.and_then(|key| properties.iter()
                    .find(|(name, _)| name == key).map(|(_, value)| value.clone())));
                let parsed = expected.each_ref().map(|value| value.as_deref().and_then(CalendarDay::parse));
                let editable = parsed[0] == Some(row.start)
                    && (expected[1].is_none() && row.end == row.start || parsed[1] == Some(row.end));
                let grip = self.scaled(7).max(1).min((right - left) / 3);
                let adjustment = on_bar.then(|| {
                    if parsed[1].is_some() && row.start == row.visible_start && point[0] < left + grip {
                        ScheduleAdjustment::Start
                    } else if keys[1].is_some() && row.end == row.visible_end && point[0] >= right - grip {
                        ScheduleAdjustment::End
                    } else {
                        ScheduleAdjustment::Move
                    }
                });
                let drag = TimelineDrag {
                    note: row.row.id, month, grid: grid.fingerprint(), press: point, current: point,
                    threshold: self.scaled(4).max(2), moved: false, adjustment, expected, editable,
                };
                unsafe {
                    SetCapture(self.hwnd);
                    if GetCapture() == self.hwnd
                        && let Some(WorkspaceView::Kanban(view)) = &mut self.extras.workspace
                    { view.timeline_drag = Some(drag); }
                }
            }
            WM_MOUSEMOVE | WM_LBUTTONUP => {
                let Some(WorkspaceView::Kanban(view)) = &mut self.extras.workspace else { return Ok(true) };
                let Some(drag) = view.timeline_drag.as_mut() else { return Ok(true) };
                drag.update(point);
                if msg == WM_MOUSEMOVE {
                    unsafe { InvalidateRect(self.hwnd, &area, 0); }
                    return Ok(true);
                }
                let Some(drag) = view.timeline_drag.take() else { return Ok(true) };
                let captured = unsafe { GetCapture() == self.hwnd };
                unsafe {
                    if captured { ReleaseCapture(); }
                    InvalidateRect(self.hwnd, &area, 0);
                }
                let Some(grid) = grid.filter(|grid| grid.fingerprint() == drag.grid && month == drag.month)
                else { return Ok(true) };
                if !captured { return Ok(true); }
                if !drag.moved {
                    unsafe { self.open_import(drag.note, "已開啟時間軸筆記。"); }
                    return Ok(true);
                }
                if drag.adjustment.is_none() || drag.delta() == 0
                    || !(grid.chart.left..grid.chart.right).contains(&point[0]) || grid.row_at(point[1]).is_none()
                {
                    return Ok(true);
                }
                let (start, end) = drag.dates().ok_or("日期必須有效，且起始日不能晚於結束日。已保留原日期。")?;
                let Some(saved) = &view.saved_view else { return Ok(true) };
                let (id, title, saved) = (view.id, view.title.clone(), saved.clone());
                self.notebook.set_collection_row_dates(id, &saved, drag.note,
                    drag.expected.each_ref().map(|raw| raw.as_deref()), [Some(start), end])?;
                unsafe {
                    self.reopen_collection_view(id, title, saved.id)?;
                    label(self.widgets.status, "已儲存時間軸日期。");
                }
            }
            WM_MOUSEWHEEL => return Ok(view.timeline_drag.is_some()),
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn timeline_grid(&self, area: RECT, month: Month) -> Option<TimelineGrid> {
        let gap = self.scaled(8);
        let left = area.left + gap;
        let right = area.right - gap;
        let chart = RECT {
            left: left + ((right - left) / 3).min(self.scaled(230)),
            top: area.top + self.scaled(54),
            right,
            bottom: area.bottom - gap,
        };
        if chart.right - chart.left < month.days() as i32 || chart.bottom <= chart.top { return None; }
        Some(TimelineGrid { chart, label_left: left, row_step: self.scaled(36).max(1) })
    }

    pub(super) unsafe fn paint_collection_timeline(
        &self, dc: HDC, area: RECT, view: &Kanban, month: Month, rows: &[ScheduledRow],
    ) {
        let Some(grid) = self.timeline_grid(area, month) else { return };
        let days = month.days();
        let mut heading = RECT { left: grid.label_left, top: area.top + self.scaled(5), right: grid.chart.right,
            bottom: grid.chart.top };
        unsafe {
            draw_text(dc, self.fonts[4], INK, &format!("{} 年 {} 月", month.year, month.month),
                &mut heading, DT_SINGLELINE);
            if rows.is_empty() {
                let mut empty = RECT { left: grid.label_left, top: grid.chart.top,
                    right: grid.chart.right, bottom: grid.chart.bottom };
                draw_text(dc, self.fonts[4], SOFT, "這個月沒有符合日期條件的資料列。", &mut empty, DT_SINGLELINE);
                return;
            }
            fill(dc, &grid.chart, PAPER);
            let daily_labels = (grid.chart.right - grid.chart.left) / days as i32 >= self.scaled(26);
            for day in 0..days {
                let left = grid.day_edge(day, days);
                let grid_line = RECT { left, top: grid.chart.top, right: left + 1, bottom: grid.chart.bottom };
                fill(dc, &grid_line, MINT);
                if daily_labels || day % 7 == 0 {
                    let span = if daily_labels { 1 } else { 7 };
                    let mut tick = RECT { left: left + self.scaled(3), top: grid.chart.top - self.scaled(23),
                        right: grid.day_edge((day + span).min(days), days), bottom: grid.chart.top };
                    draw_text(dc, self.fonts[4], TEAL, &(day + 1).to_string(), &mut tick, DT_SINGLELINE | DT_END_ELLIPSIS);
                }
            }
            for (offset, event) in rows.iter().skip(view.row).take(grid.row_count()).enumerate() {
                let top = grid.row_top(offset);
                let bottom = grid.row_top(offset + 1) - 1;
                let mut label = RECT { left: grid.label_left, top, right: grid.chart.left - self.scaled(6), bottom };
                fill(dc, &label, PAPER);
                label.left += self.scaled(5);
                draw_text(dc, self.fonts[4], INK, &event.row.title, &mut label, DT_SINGLELINE | DT_END_ELLIPSIS);
                label.top += self.scaled(17);
                draw_text(dc, self.fonts[4], SOFT, &event.status, &mut label, DT_SINGLELINE | DT_END_ELLIPSIS);
                let preview = view.timeline_drag.as_ref().filter(|drag| drag.note == event.row.id
                    && drag.moved && drag.adjustment.is_some() && drag.grid == grid.fingerprint());
                let dates = preview.and_then(TimelineDrag::dates);
                let color = match (preview, dates) {
                    (Some(_), Some(_)) => SOFT,
                    (Some(_), None) => super::super::color(152, 67, 51),
                    _ => TEAL,
                };
                let (from, through, start, end) = if let Some((start, end)) = dates {
                    let (lower, upper) = month.bounds();
                    let (lower, upper) = view.saved_view.as_ref().map_or((lower, upper), |saved| {
                        let p = &saved.presentation;
                        (p.calendar_from.map_or(lower, |start| start.max(lower)),
                         p.calendar_until.map_or(upper, |end| end.min(upper)))
                    });
                    let end = end.unwrap_or(start);
                    let (from, through) = (start.max(lower), end.min(upper));
                    if from > through { continue; }
                    (from, through, start, end)
                } else { (event.visible_start, event.visible_end, event.start, event.end) };
                let left = grid.day_edge(from.day() - 1, days);
                let right = grid.day_edge(through.day(), days);
                let mut bar = RECT { left, right, top: top + self.scaled(6), bottom: bottom - self.scaled(5) };
                fill(dc, &bar, color);
                let mark = self.scaled(3).max(1).min(bar.right - bar.left);
                if start < from {
                    fill(dc, &RECT { right: bar.left + mark, ..bar }, MINT);
                }
                if end > through {
                    fill(dc, &RECT { left: bar.right - mark, ..bar }, MINT);
                }
                if bar.right - bar.left >= self.scaled(58) {
                    let left = if start < from { "‹ " } else { "" };
                    let right = if end > through { " ›" } else { "" };
                    let ends = format!("{left}{}–{}{right}", from.day(), through.day());
                    bar.left += self.scaled(4);
                    draw_text(dc, self.fonts[4], PAPER, &ends, &mut bar, DT_SINGLELINE | DT_END_ELLIPSIS);
                }
            }
        }
    }

    fn calendar_grid(&self, area:RECT) -> RECT {
        RECT {left:area.left+self.scaled(8),top:area.top+self.scaled(53),
              right:area.right-self.scaled(8),bottom:area.bottom-self.scaled(8)}
    }

    pub(super) unsafe fn paint_collection_calendar(&self,dc:HDC,area:RECT,month:Month,days:&BTreeMap<usize,Vec<Row>>) {
        let grid = self.calendar_grid(area);
        let width = grid.right-grid.left;
        let height = grid.bottom-grid.top;
        if width < 7 || height < 6 { return; }
        let mut title = RECT { left:grid.left,top:area.top+self.scaled(6),right:grid.right,bottom:grid.top };
        unsafe {
            draw_text(dc,self.fonts[4],INK,&format!("{} 年 {} 月",month.year,month.month),&mut title,DT_SINGLELINE);
            for (column,weekday) in ["日","一","二","三","四","五","六"].iter().enumerate() {
                let mut rect = RECT {left:grid.left+width*column as i32/7+self.scaled(5),
                    top:grid.top-self.scaled(20),right:grid.left+width*(column+1) as i32/7,bottom:grid.top};
                draw_text(dc,self.fonts[4],TEAL,weekday,&mut rect,DT_SINGLELINE);
            }
            for slot in 0..42 {
                let mut cell = RECT {left:grid.left+width*(slot%7)/7,top:grid.top+height*(slot/7)/6,
                    right:grid.left+width*(slot%7+1)/7-1,bottom:grid.top+height*(slot/7+1)/6-1};
                fill(dc,&cell,PAPER);
                let Some(day) = (slot as usize+1).checked_sub(month.sunday_offset())
                    .filter(|d|*d>0 && *d<=month.days()) else { continue };
                let rows = days.get(&day);
                if rows.is_some() { fill(dc,&cell,MINT); }
                cell.left += self.scaled(5);
                let date = if let Some(rows)=rows { format!("{day}　({})",rows.len()) } else { day.to_string() };
                draw_text(dc,self.fonts[4],INK,&date,&mut cell,DT_SINGLELINE | DT_END_ELLIPSIS);
                if let Some(row) = rows.and_then(|rows|rows.first()) && height/6 >= self.scaled(35) {
                    cell.top += self.scaled(17);
                    draw_text(dc,self.fonts[4],TEAL,&row.title,&mut cell,DT_SINGLELINE | DT_END_ELLIPSIS);
                }
            }
        }
    }

    pub(super) unsafe fn paint_collection_week(
        &self,dc:HDC,area:RECT,week:CalendarDay,days:&BTreeMap<CalendarDay,Vec<Row>>,
    ) {
        let grid = self.calendar_grid(area);
        let width = grid.right-grid.left;
        if width<7 || grid.bottom<=grid.top { return; }
        let end = week.shifted(6).map(CalendarDay::iso).unwrap_or_else(||"9999-12-31".into());
        let mut title = RECT {left:grid.left,top:area.top+self.scaled(6),right:grid.right,bottom:grid.top};
        unsafe {
            draw_text(dc,self.fonts[4],INK,&format!("{} 至 {end}",week.iso()),&mut title,DT_SINGLELINE);
            for (column,weekday) in ["一","二","三","四","五","六","日"].iter().enumerate() {
                let Some(date) = week.shifted(column as i8) else { continue };
                let left = grid.left+width*column as i32/7;
                let right = grid.left+width*(column+1) as i32/7;
                let mut heading = RECT {left:left+self.scaled(5),top:grid.top-self.scaled(20),right,bottom:grid.top};
                draw_text(dc,self.fonts[4],TEAL,&format!("{weekday}　{}/{}",date.month().month,date.day()),
                    &mut heading,DT_SINGLELINE|DT_END_ELLIPSIS);
                let mut cell = RECT {left,top:grid.top,right:right-1,bottom:grid.bottom};
                fill(dc,&cell,if days.contains_key(&date) { MINT } else { PAPER });
                if let Some(rows) = days.get(&date) {
                    cell.left += self.scaled(5);
                    let step = self.scaled(23).max(1);
                    draw_text(dc,self.fonts[4],TEAL,&format!("{} 篇",rows.len()),&mut cell,DT_SINGLELINE|DT_END_ELLIPSIS);
                    for row in rows {
                        cell.top += step;
                        if cell.top+step>cell.bottom { break; }
                        draw_text(dc,self.fonts[4],INK,&row.title,&mut cell,DT_SINGLELINE|DT_END_ELLIPSIS);
                    }
                }
            }
        }
    }

    pub(super) unsafe fn paint_collection_list(&self,dc:HDC,area:RECT,view:&Kanban) {
        let (left,right,top,step) = self.collection_table_rect(area);
        if right<=left { return; }
        let visible = ((area.bottom-top)/step).max(0) as usize;
        unsafe {
            if visible>0 && view.lanes.is_empty() {
                let mut row = RECT {left,top,right,bottom:top+step};
                draw_text(dc,self.fonts[4],TEAL,"沒有符合條件的資料列。",&mut row,DT_SINGLELINE);
            }
            for (index, line) in view.table_lines().skip(view.row).take(visible).enumerate() {
                let y = top+index as i32*step;
                let mut row = RECT {left,top:y,right,bottom:y+step-1};
                match line {
                    CollectionLine::Heading(group, count, folded) => self.paint_collection_fold_header(dc, row, group, count, folded),
                    CollectionLine::Note(group, note) => {
                        fill(dc,&row,if index%2==0 { PAPER } else { MINT });
                        row.left += self.scaled(8);
                        draw_text(dc,self.fonts[4],INK,&format!("{}　·　{}",note.title,group.label()),&mut row,DT_SINGLELINE|DT_END_ELLIPSIS);
                    }
                }
            }
        }
    }

    pub(super) unsafe fn click_collection_checkbox(&mut self, lp: LPARAM) -> Result<()> {
        let Some(WorkspaceView::Kanban(view)) = &self.extras.workspace else { return Ok(()) };
        let Some((key,values)) = &view.checkbox else { return Ok(()) };
        let area = unsafe { self.board_area() };
        let x = lp as u16 as i16 as i32;
        let y = (lp >> 16) as u16 as i16 as i32;
        let (left,right,top,step) = self.collection_table_rect(area);
        let checkbox_left = right - (right-left)/4;
        let first_row = top+step;
        if x < checkbox_left || x >= right || y < first_row || y >= area.bottom { return Ok(()); }
        let offset = ((y-first_row)/step) as usize;
        if offset >= ((area.bottom-first_row)/step).max(0) as usize { return Ok(()); }
        let Some((status,row)) = view.lanes.iter().flat_map(|(status,rows)| {
            rows.iter().map(move |row| (status,row))
        }).nth(view.row+offset) else { return Ok(()) };
        let checked = self.notebook.toggle_collection_checkbox(
            view.id,row.id,status.0.as_deref().unwrap_or_default(),key,values.get(&row.id).map(String::as_str),
        )?;
        let (id,title,saved) = (view.id,view.title.clone(),view.saved_view.clone());
        unsafe {
            self.open_collection_layout(id,title,saved)?;
            label(self.widgets.status, if checked { "已勾選" } else { "已取消勾選" });
        }
        Ok(())
    }

    pub(super) fn collection_table_rect(&self, area: RECT) -> (i32,i32,i32,i32) {
        (area.left+self.scaled(8),area.right-self.scaled(8),
         area.top+self.scaled(6),self.scaled(34).max(1))
    }

    pub(super) unsafe fn paint_collection_table(&self, dc:HDC, area:RECT, view:&Kanban) {
        unsafe {
            let (left,right,top,step) = self.collection_table_rect(area);
            if left >= right { return; }
            let divider = left + (right-left)/3;
            let checkbox_left = if view.checkbox.is_some() { right-(right-left)/4 } else { right };
            let draw_row = |y:i32, status:&str, title:&str,checked:&str,header:bool| {
                let mut row = RECT {left,top:y,right,bottom:(y+step-1).min(area.bottom)};
                fill(dc,&row,if header { MINT } else { PAPER });
                row.left += self.scaled(6);
                row.right = divider;
                let flags = DT_SINGLELINE | DT_END_ELLIPSIS;
                draw_text(dc,self.fonts[4],TEAL,status,&mut row,flags);
                row.left = divider+self.scaled(6);
                row.right = checkbox_left;
                draw_text(dc,self.fonts[4],INK,title,&mut row,flags);
                if view.checkbox.is_some() {
                    row.left = checkbox_left+self.scaled(6);
                    row.right = right;
                    draw_text(dc,self.fonts[4],TEAL,checked,&mut row,flags);
                }
            };
            let group = view.saved_view.as_ref().and_then(|view|view.presentation.group_key.as_deref()).unwrap_or("狀態");
            draw_row(top,group,"筆記標題",view.checkbox.as_ref().map_or("",|(key,_)|key),true);
            if view.lanes.is_empty() {
                draw_row(top+step,"","沒有符合條件的資料列。","",false);
                return;
            }
            let visible = ((area.bottom-top-step)/step).max(0) as usize;
            for (offset, line) in view.table_lines().skip(view.row).take(visible).enumerate() {
                let y = top+(offset as i32+1)*step;
                match line {
                    CollectionLine::Heading(group, count, folded) => {
                        let rect = RECT {left,top:y,right,bottom:y+step-1};
                        self.paint_collection_fold_header(dc, rect, group, count, folded);
                    }
                    CollectionLine::Note(group, note) => {
                        let state = view.checkbox.as_ref().map_or("",|(_,values)| {
                            checkbox_label(values.get(&note.id).map(String::as_str))
                        });
                        draw_row(y, &group.label(), &note.title, state, false);
                    }
                }
            }
        }
    }
}


fn template_preview(template: &CollectionTemplate) -> Result<String> {
    let mut out = format!("範本　{}\r\n預設標題　{}\r\n集合狀態　{}\r\n", template.name, template.default_title, template.status);
    if template.fields.is_empty() { out.push_str("型別欄位　未選取\r\n"); }
    for field in &template.fields {
        write!(out, "{}　{}\r\n", field.key, field.value)?;
    }
    if !template.resources.is_empty() {
        out.push_str("\r\n以下項目每次建立時須明確選擇是否套用。附件會複製目前已存的內容，PDF 批註留在原檔。\r\n");
        for item in &template.resources { writeln!(out, "{}", item.label())?; }
    }
    write!(out, "\r\n正文\r\n{}", template.body)?;
    if !template.children.is_empty() {
        out.push_str("\r\n\r\n選定的子頁依以下階層各自建立新筆記。");
        for (path, child) in template.child_paths().into_iter().zip(&template.children) {
            write!(out, "\r\n\r\n子頁　{path}\r\n{}", child.body)?;
        }
    }
    Ok(out)
}

fn filter_toggle_label(mode: FilterMode) -> &'static str {
    match mode {
        FilterMode::All => "目前全部相符，切換為任一相符",
        FilterMode::Any => "目前任一相符，切換為全部相符",
    }
}

fn toggle_filter_mode(mode: &mut FilterMode) {
    *mode = if *mode == FilterMode::All { FilterMode::Any } else { FilterMode::All };
}

fn filter_summary(rule: &FilterRule) -> String {
    let value = rule.value.lines().next().unwrap_or_default().chars().take(60).collect::<String>();
    format!("篩選　{}　{}　{value}", rule.label(), rule.operator.label())
}

fn parse_day(raw: &str) -> Result<Option<CalendarDay>> {
    if raw.trim().is_empty() { Ok(None) }
    else { Ok(Some(CalendarDay::parse(raw).ok_or("請輸入有效的 YYYY-MM-DD 日期。")?)) }
}

pub(super) fn checkbox_label(value: Option<&str>) -> &'static str {
    match value.map(|raw| PropertyType::Checkbox.normalized(raw)) {
        None => "未設定",
        Some(Some(value)) if value == "true" => "已勾選",
        Some(Some(_)) => "未勾選",
        Some(None) => "舊文字",
    }
}


fn current_month() -> Month {
    let mut now = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut now); }
    Month {year:now.wYear,month:now.wMonth}
}


fn current_day() -> Result<CalendarDay> { CalendarDay::local_today() }
