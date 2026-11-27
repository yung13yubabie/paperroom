use super::*;
use crate::store::board_objects::BoardItem;
use crate::store::mind_map::{MindNode, TITLE_LIMIT};

fn node_caption(node: &MindNode) -> String {
    let title: String = node.title.chars().take(56).collect();
    format!("{}{}  (#{})", if node.parent.is_none() { "根 · " } else { "" },
        title, node.id)
}

fn topic_rect(rect: RECT, scale: impl Fn(i32) -> i32) -> RECT {
    RECT {
        left: rect.left + scale(7), right: rect.right - scale(7),
        top: rect.top + scale(23), bottom: rect.bottom - scale(23),
    }
}

impl App {
    pub(super) unsafe fn add_mind_map_root_ui(&mut self) -> Result<()> {
        unsafe { self.add_mind_node_ui(None) }
    }

    unsafe fn add_mind_node_ui(&mut self, parent: Option<&MindNode>) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let caption = if parent.is_some() { "新增子節點" } else { "新增心智圖根節點" };
            if let Some(value) = crate::dialog::prompt(self.hwnd, caption, "主題", "", TITLE_LIMIT)? {
                let id = self.notebook.add_mind_node(board, parent, &value)?;
                self.refresh_board()?;
                self.reveal_mind_node(id);
                label(self.widgets.status, if parent.is_some() { "已新增子節點。" } else { "已新增心智圖根節點。" });
            }
            Ok(())
        }
    }

    unsafe fn reveal_mind_node(&mut self, id: i64) {
        unsafe {
            let Some(object) = self.extras.objects.iter().find(|object| object.id == id) else {
                return;
            };
            let area = self.board_area();
            let width = (area.right - area.left) * 96 / self.dpi as i32;
            if object.x < self.extras.board_scroll_x || object.x + 232 > self.extras.board_scroll_x + width {
                self.extras.board_scroll_x = (object.x - 8).max(0);
            }
            let height = (area.bottom - area.top) * 96 / self.dpi as i32;
            if object.y < self.extras.scroll || object.y + 114 > self.extras.scroll + height {
                self.extras.scroll = (object.y - 8).max(0);
            }
            InvalidateRect(self.hwnd, &area, 1);
        }
    }

    pub(super) unsafe fn mind_map_picker_ui(&mut self) -> Result<()> {
        unsafe {
            let nodes = self.notebook.mind_nodes_on(self.board_id())?;
            if nodes.is_empty() {
                label(self.widgets.status, "請先新增心智圖根節點。");
                return Ok(());
            }
            let captions: Vec<_> = nodes.iter().map(node_caption).collect();
            let choices: Vec<_> = captions.iter().enumerate().map(|(i, entry)| (i + 1, entry.as_str())).collect();
            let id = popup(self.hwnd, self.widgets.links, None, &choices);
            if let Some(node) = id.checked_sub(1).and_then(|id| nodes.get(id)) {
                self.reveal_mind_node(node.id);
                self.mind_node_menu(node, None)?;
            }
            Ok(())
        }
    }

    pub(super) unsafe fn edit_mind_node_ui(&mut self, node: &MindNode) -> Result<()> {
        unsafe {
            let board = self.board_id();
            if let Some(value) = crate::dialog::prompt(
                self.hwnd, "編輯心智圖節點", "主題", &node.title, TITLE_LIMIT,
            )? {
                self.notebook.edit_mind_node(board, node, &value)?;
                self.refresh_board()?;
                label(self.widgets.status, "已保存心智圖主題。");
            }
            Ok(())
        }
    }

    unsafe fn reparent_mind_node_ui(&mut self, node: &MindNode) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let nodes: Vec<_> = self.notebook.mind_nodes_on(board)?.into_iter()
                .filter(|candidate| candidate.id != node.id).collect();
            let captions: Vec<_> = nodes.iter().map(node_caption).collect();
            let mut choices = vec![(1, "設為根節點")];
            choices.extend(captions.iter().enumerate().map(|(i, entry)| (i + 2, entry.as_str())));
            let selected = popup(self.hwnd, self.widgets.links, None, &choices);
            let parent = match selected {
                0 => return Ok(()),
                1 => None,
                index => Some(nodes.get(index - 2).ok_or("找不到心智圖節點。")?),
            };
            self.notebook.reparent_mind_node(board, node, parent)?;
            self.refresh_board()?;
            label(self.widgets.status, "已更新父節點，保留節點位置與子分支。");
            Ok(())
        }
    }

    pub(super) unsafe fn mind_node_menu(&mut self, node: &MindNode, point: Option<POINT>) -> Result<()> {
        unsafe {
            match popup(self.hwnd, self.widgets.links, point, &[
                (1, "新增子節點…"), (2, "編輯節點…"), (3, "變更父節點…"),
                (4, "選取整個分支"), (5, "刪除葉節點…"), (6, "匯出分支 JSON Canvas…"),
            ]) {
                1 => self.add_mind_node_ui(Some(node))?,
                2 => self.edit_mind_node_ui(node)?,
                3 => self.reparent_mind_node_ui(node)?,
                4 => {
                    let branch = self.notebook.mind_branch(self.board_id(), node)?;
                    self.cancel_board_selection();
                    self.refresh_board()?;
                    let count = branch.len();
                    self.extras.board_selected = branch.into_iter().map(BoardItem::Object).collect();
                    InvalidateRect(self.hwnd, &self.board_area(), 1);
                    label(self.widgets.status, &format!("已選取分支的 {count} 個節點。可一起拖曳。"));
                }
                5 => {
                    if self.notebook.mind_branch(self.board_id(), node)?.len() != 1 {
                        return Err("先移動或逐一刪除子節點，才能刪除這個節點。".into());
                    }
                    let question = format!("刪除「{}」？", node.title);
                    if message(self.hwnd, "刪除葉節點", &question, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES {
                        self.notebook.remove_mind_leaf(self.board_id(), node)?;
                        self.refresh_board()?;
                        label(self.widgets.status, "已刪除葉節點。");
                    }
                }
                6 => {
                    if let Some(path) = file_dialog(self.hwnd, "JSON Canvas\0*.canvas\0\0",
                        "canvas", Some("mind-map.canvas"))?
                    {
                        let count = self.notebook.export_mind_branch(self.board_id(), node, &path)?;
                        label(self.widgets.status, &format!("已匯出 {count} 個心智圖節點為 JSON Canvas。"));
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }

    pub(super) unsafe fn paint_mind_map_edges(&self, dc: HDC, area: RECT) {
        unsafe {
            if self.extras.mind_map_nodes.iter().all(|node| node.parent.is_none()) { return; }
            let pen = CreatePen(PS_SOLID, self.scaled(2).max(1), color(53, 129, 137));
            if pen.is_null() { return; }
            let previous = SelectObject(dc, pen);
            let points: HashMap<_, _> = self.extras.objects.iter().map(|object| (
                object.id, (self.board_screen_x(area, object.x + 116),
                    area.top + self.scaled(object.y - self.extras.scroll + 57)),
            )).collect();
            for node in &self.extras.mind_map_nodes {
                let (Some(&(ax, ay)), Some(&(bx, by))) = (
                    node.parent.and_then(|id| points.get(&id)), points.get(&node.id),
                ) else { continue; };
                let (dx, dy) = (f64::from(bx - ax), f64::from(by - ay));
                let length = dx.hypot(dy);
                if length < 1.0 { continue; }
                let (ux, uy) = (dx / length, dy / length);
                let factor = (dx.abs() / f64::from(self.scaled(109)).max(1.0))
                    .max(dy.abs() / f64::from(self.scaled(34)).max(1.0));
                if factor <= 1.0 { continue; }
                let x = bx - (dx / factor).round() as i32;
                let y = by - (dy / factor).round() as i32;
                let end = f64::from(self.scaled(9));
                let wing = f64::from(self.scaled(4));
                MoveToEx(dc, ax, ay, null_mut());
                LineTo(dc, x, y);
                MoveToEx(dc, x, y, null_mut());
                LineTo(dc, x - (ux * end + uy * wing).round() as i32,
                    y - (uy * end - ux * wing).round() as i32);
                MoveToEx(dc, x, y, null_mut());
                LineTo(dc, x - (ux * end - uy * wing).round() as i32,
                    y - (uy * end + ux * wing).round() as i32);
            }
            SelectObject(dc, previous);
            DeleteObject(pen);
        }
    }

    pub(super) unsafe fn paint_mind_map_node(&self, dc: HDC, rect: RECT, node: &MindNode) {
        unsafe {
            let root = node.parent.is_none();
            let selected = self.extras.board_selected.contains(&BoardItem::Object(node.id));
            let rect = topic_rect(rect, |v| self.scaled(v));
            let brush = CreateSolidBrush(if root { color(28, 104, 112) } else { color(230, 244, 245) });
            let pen = CreatePen(PS_SOLID, self.scaled(if selected { 3 } else { 1 }).max(1),
                if selected { color(42, 80, 151) } else { color(53, 129, 137) });
            if !brush.is_null() && !pen.is_null() {
                let previous_pen = SelectObject(dc, pen);
                let previous_brush = SelectObject(dc, brush);
                let corner = self.scaled(18);
                RoundRect(dc, rect.left, rect.top, rect.right, rect.bottom, corner, corner);
                SelectObject(dc, previous_brush);
                SelectObject(dc, previous_pen);
            }
            if !pen.is_null() { DeleteObject(pen); }
            if !brush.is_null() { DeleteObject(brush); }
            let mut caption = RECT {
                left: rect.left + self.scaled(12), right: rect.right - self.scaled(12),
                top: rect.top + self.scaled(8), bottom: rect.bottom - self.scaled(8),
            };
            draw_text(dc, self.fonts[0], if root { PAPER } else { INK }, &node.title, &mut caption,
                DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX | DT_CENTER);
        }
    }
}
