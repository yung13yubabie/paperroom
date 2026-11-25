use super::*;
use crate::store::collection_group_folds::GroupValue;

pub(super) enum CollectionLine<'a> {
    Heading(&'a GroupValue, usize, bool),
    Note(&'a GroupValue, &'a Row),
}

impl Kanban {
    pub(super) fn is_folded(&self, group: &GroupValue) -> bool {
        self.group_folds.as_ref().is_some_and(|folds| folds.contains(group))
    }

    pub(super) fn expanded_lanes(&self) -> impl Iterator<Item = &(GroupValue, Vec<Row>)> {
        self.lanes.iter().filter(|(group, _)| !self.is_folded(group))
    }

    pub(super) fn table_lines(&self) -> impl Iterator<Item = CollectionLine<'_>> {
        self.lanes.iter().flat_map(|(group, rows)| {
            let folded = self.is_folded(group);
            let heading = self.group_folds.is_some().then(|| CollectionLine::Heading(group, rows.len(), folded));
            let visible = if folded { &[][..] } else { rows.as_slice() };
            heading.into_iter().chain(visible.iter().map(|row| CollectionLine::Note(group, row)))
        })
    }

    pub(super) fn fold_heading(group: &GroupValue, count: usize, folded: bool) -> String {
        let marker = if folded { "▸" } else { "▾" };
        format!("{marker} {}  ·  {count}", group.fold_label())
    }
}

impl App {
    pub(super) unsafe fn collection_fold_menu(&mut self, id: i64, title: String) -> Result<()> {
        let Some(WorkspaceView::Kanban(view)) = &self.extras.workspace else { return Ok(()); };
        let (Some(saved), Some(folds)) = (&view.saved_view, &view.group_folds) else {
            return Err("請先開啟具名看板，或依屬性分組的具名表格。".into());
        };
        let mut labels = vec!["展開所有群組".to_owned()];
        labels.extend(view.lanes.iter().map(|(group, rows)| {
            let action = if folds.contains(group) { "展開" } else { "收合" };
            format!("{action} {}  ·  {}", group.fold_label(), rows.len())
        }));
        let Some(choice) = (unsafe { choose(self.hwnd, self.widgets.links, &labels, String::clone) }) else { return Ok(()); };
        let change = choice.checked_sub(1).map(|index| &view.lanes[index].0)
            .map(|group| (group, !folds.contains(group)));
        self.notebook.set_collection_group_fold(id, saved, folds, change)?;
        unsafe { self.open_collection_layout(id, title, Some(saved.clone())) }
    }

    pub(super) unsafe fn click_collection_fold(&mut self, area: RECT, x: i32, y: i32) -> Result<bool> {
        let Some(WorkspaceView::Kanban(view)) = &self.extras.workspace else { return Ok(false); };
        let (Some(saved), Some(folds)) = (&view.saved_view, &view.group_folds) else { return Ok(false); };
        let group = if view.is_table() {
            let (left, right, top, step) = self.collection_table_rect(area);
            let first = view.first_table_line(top, step);
            if x < left || x >= right || y < first || y >= first + ((area.bottom - first) / step) * step {
                return Ok(false);
            }
            let index = view.row + ((y - first) / step) as usize;
            match view.table_lines().nth(index) {
                Some(CollectionLine::Heading(group, _, _)) => Some(group),
                _ => None,
            }
        } else {
            let width = (area.right - area.left) / 3;
            if width <= 0 || x < area.left || y < area.top + self.scaled(10) || y >= area.top + self.scaled(42) {
                return Ok(false);
            }
            let column = ((x - area.left) / width) as usize;
            let offset = (x - area.left) % width;
            if column >= 3 || offset < self.scaled(7) || offset >= width - self.scaled(9) { return Ok(false); }
            view.lanes.get(view.column + column).map(|(group, _)| group)
        };
        let Some(group) = group else { return Ok(false); };
        self.notebook.set_collection_group_fold(view.id, saved, folds, Some((group, !folds.contains(group))))?;
        let (id, title, saved) = (view.id, view.title.clone(), saved.clone());
        unsafe { self.open_collection_layout(id, title, Some(saved))?; }
        Ok(true)
    }

    pub(super) unsafe fn paint_collection_fold_header(
        &self, dc: HDC, bounds: RECT, group: &GroupValue, count: usize, folded: bool,
    ) {
        let mut text = bounds;
        text.left += self.scaled(6);
        unsafe {
            fill(dc, &bounds, MINT);
            draw_text(dc, self.fonts[4], TEAL, &Kanban::fold_heading(group, count, folded),
                &mut text, DT_SINGLELINE | DT_END_ELLIPSIS);
        }
    }
}
