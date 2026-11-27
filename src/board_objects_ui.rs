use super::*;
use crate::store::board_images::ImageSize;
use crate::store::board_layout::{BoardArrangement, BoardLayoutRect};
use crate::store::board_objects::{BoardItem, BoardPosition, Object, TEXT_LIMIT};

pub(super) struct BoardBatchDrag {
    board: i64,
    start: (i32, i32),
    expected: Vec<BoardPosition>,
    delta: (i32, i32),
    drop_hover: Option<bool>,
}

pub(super) struct BoardDropTarget {
    source: i64,
    destination: i64,
    title: String,
}

fn drop_target_bounds(board: RECT, scale: impl Fn(i32) -> i32) -> Option<RECT> {
    let left = board.left + scale(120);
    let right = (left + scale(320)).min(board.right);
    (right - left >= scale(140)).then_some(RECT {
        left, right, top: board.top - scale(38), bottom: board.top - scale(6),
    })
}

fn over_drop_target(rect: RECT, x: i32, y: i32) -> bool {
    (rect.left..rect.right).contains(&x) && (rect.top..rect.bottom).contains(&y)
}

impl App {
    fn board_selection_snapshot(&self) -> Vec<BoardPosition> {
        self.extras.cards.iter().map(|card| BoardPosition {
            item: BoardItem::Card(card.id), x: card.x, y: card.y, version: None,
        }).chain(self.extras.objects.iter().map(|object| BoardPosition {
            item: BoardItem::Object(object.id), x: object.x, y: object.y, version: Some(object.version),
        })).filter(|entry| self.extras.board_selected.contains(&entry.item)).collect()
    }

    pub(super) unsafe fn board_selection_menu(&mut self) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let expected = self.board_selection_snapshot();
            if expected.is_empty() {
                label(self.widgets.status, "按住 Ctrl 點選要移動的白板物件。");
                return Ok(());
            }
            let Some(value) = crate::dialog::prompt(self.hwnd, "移動選取的物件",
                "輸入水平、垂直位移（dx,dy，可為負數）", "0,0", 32)? else { return Ok(()) };
            let (dx, dy) = value.split_once(',').ok_or("請用逗號分隔兩個整數。")?;
            let dx = dx.trim().parse::<i32>().map_err(|_| "水平位移須為整數。")?;
            let dy = dy.trim().parse::<i32>().map_err(|_| "垂直位移須為整數。")?;
            self.finish_board_selection_move(board, &expected, (dx, dy))
        }
    }

    pub(super) unsafe fn board_layout_menu(&mut self) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let expected: Vec<_> = self.board_selection_snapshot().into_iter().map(|position| {
                let size = match position.item {
                    BoardItem::Card(id) => self.extras.cards.iter().find(|card| card.id == id)
                        .map_or(ImageSize::CARD, |card| card.size()),
                    BoardItem::Object(_) => ImageSize::CARD,
                };
                BoardLayoutRect { position, size }
            }).collect();
            if expected.len() < 2 {
                label(self.widgets.status, "按住 Ctrl 點選至少兩個物件，即可對齊或排列。");
                return Ok(());
            }
            let operations = [
                ("靠左對齊", BoardArrangement::Left), ("水平置中", BoardArrangement::CenterX),
                ("靠右對齊", BoardArrangement::Right), ("靠上對齊", BoardArrangement::Top),
                ("垂直置中", BoardArrangement::CenterY), ("靠下對齊", BoardArrangement::Bottom),
                ("水平等距", BoardArrangement::GapX), ("垂直等距", BoardArrangement::GapY),
            ];
            let count = if expected.len() >= 3 { operations.len() } else { 6 };
            let choices: Vec<_> = operations[..count].iter().enumerate()
                .map(|(index, (name, _))| (index + 1, *name)).collect();
            let selected = popup(self.hwnd, self.widgets.links, None, &choices);
            if let Some((_, operation)) = selected.checked_sub(1).and_then(|index| operations[..count].get(index)) {
                let result = self.notebook.arrange_board_selection(board, &expected, *operation);
                let refreshed = self.refresh_board();
                let moved = result?;
                refreshed?;
                if moved == 0 {
                    label(self.widgets.status, "物件位置已符合所選排列。");
                } else {
                    label(self.widgets.status, &format!("已排列 {moved} 個白板物件。"));
                }
            }
            Ok(())
        }
    }

    pub(super) unsafe fn undo_board_layout_ui(&mut self) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let undo = self.notebook.board_layout_undo(board)?;
            if undo.is_empty() {
                label(self.widgets.status, "這張白板沒有可復原的排列。");
                return Ok(());
            }
            let question = format!("復原這張白板最近一次排列的 {} 個物件位置？", undo.len());
            if message(self.hwnd, "復原最近一次排列", &question, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                return Ok(());
            }
            let result = self.notebook.undo_board_layout(board, &undo);
            let refreshed = self.refresh_board();
            result?;
            refreshed?;
            label(self.widgets.status, "已復原最近一次排列。");
            Ok(())
        }
    }

    pub(super) unsafe fn reuse_selected_board_cards_menu(&mut self) -> Result<()> {
        unsafe {
            let source = self.board_id();
            let expected = self.board_selection_snapshot();
            if expected.is_empty() {
                label(self.widgets.status, "按住 Ctrl 點選要在另一張白板使用的卡片。");
                return Ok(());
            }
            if expected.iter().any(|entry| matches!(entry.item, BoardItem::Object(_))) {
                return Err("請只選取要跨板使用的筆記、PDF 摘錄或圖片卡。".into());
            }
            let boards: Vec<_> = self.notebook.list_boards()?.into_iter().filter(|board| board.id != source).collect();
            if boards.is_empty() {
                label(self.widgets.status, "另建一張白板後，就能在兩張白板使用同一張卡片。");
                return Ok(());
            }
            let choices: Vec<_> = boards.iter().enumerate()
                .map(|(index, board)| (index + 1, board.title.as_str())).collect();
            let chosen = popup(self.hwnd, self.widgets.links, None, &choices);
            if let Some(board) = chosen.checked_sub(1).and_then(|index| boards.get(index)) {
                let result = self.notebook.reuse_board_cards(source, board.id, &expected);
                let refreshed = self.refresh_board();
                let added = result?;
                refreshed?;
                let status = if added == 0 {
                    format!("「{}」已放入這 {} 張卡片。", board.title, expected.len())
                } else {
                    format!("已在「{}」放入 {} 張原卡片，已有 {} 張。", board.title, added, expected.len() - added)
                };
                label(self.widgets.status, &status);
            }
            Ok(())
        }
    }

    pub(super) unsafe fn copy_selected_layout_menu(&mut self) -> Result<()> {
        unsafe {
            let source=self.board_id();
            let positions=self.board_selection_snapshot();
            if positions.is_empty() {
                label(self.widgets.status,"按住 Ctrl 點選要複製配置的卡片。");
                return Ok(());
            }
            let selection=self.notebook.plan_selection_child(source,&positions)?;
            let boards=self.notebook.list_boards()?.into_iter().filter(|b|b.id!=source).collect::<Vec<_>>();
            if boards.is_empty() { label(self.widgets.status,"請先建立另一張目的白板。"); return Ok(()); }
            let Some(index)=navigation_ui::choose_navigation_page(self.hwnd,self.widgets.links,boards.len(),
                |i|format!("[#{}]　{}",boards[i].id,boards[i].title)) else {return Ok(())};
            let target=&boards[index];
            let plan=self.notebook.plan_copy_layout(selection,target)?;
            let Some(delta)=crate::dialog::prompt(self.hwnd,"跨白板複製配置","輸入水平、垂直位移（dx,dy，可為負數）","0,0",32)?
                else {return Ok(())};
            let (dx,dy)=delta.split_once(',').ok_or("請用逗號分隔兩個整數。")?;
            let dx=dx.trim().parse::<i32>().map_err(|_|"水平位移須為整數。")?;
            let dy=dy.trim().parse::<i32>().map_err(|_|"垂直位移須為整數。")?;
            let message_text=format!("將 {} 張卡片放到「{}」並位移（{dx}, {dy}）？兩張白板共用原筆記，分區與連線可分別編輯。",positions.len(),target.title);
            if message(self.hwnd,"確認複製白板配置",&message_text,MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2)==IDYES {
                self.notebook.copy_layout(&plan,dx,dy)?;
                label(self.widgets.status,&format!("已將 {} 張卡片與配置複製到「{}」，原白板已保留。",positions.len(),target.title));
            }
            Ok(())
        }
    }

    pub(super) unsafe fn move_selected_board_cards_menu(&mut self) -> Result<()> {
        unsafe {
            let source = self.board_id();
            let expected = self.board_selection_snapshot();
            if expected.is_empty() {
                label(self.widgets.status, "按住 Ctrl 點選要搬到另一張白板的卡片。");
                return Ok(());
            }
            if expected.iter().any(|entry| matches!(entry.item, BoardItem::Object(_))) {
                return Err("請只選取要跨板搬移的筆記、PDF 摘錄或圖片卡。".into());
            }
            let boards: Vec<_> = self.notebook.list_boards()?.into_iter()
                .filter(|board| board.id != source).collect();
            if boards.is_empty() {
                label(self.widgets.status, "請先建立另一張目的白板。");
                return Ok(());
            }
            let choices: Vec<_> = boards.iter().enumerate()
                .map(|(index, board)| (index + 1, board.title.as_str())).collect();
            let chosen = popup(self.hwnd, self.widgets.links, None, &choices);
            if let Some(board) = chosen.checked_sub(1).and_then(|index| boards.get(index)) {
                self.confirm_board_card_transfer(&BoardDropTarget {
                    source, destination: board.id, title: board.title.clone(),
                }, &expected)?;
            }
            Ok(())
        }
    }

    pub(super) unsafe fn arm_board_drop_target(&mut self) -> Result<()> {
        unsafe {
            let source = self.board_id();
            let expected = self.board_selection_snapshot();
            if expected.is_empty() {
                label(self.widgets.status, "按住 Ctrl 點選要跨板拖曳的卡片。");
                return Ok(());
            }
            if expected.iter().any(|entry| matches!(entry.item, BoardItem::Object(_))) {
                return Err("請只選取要跨板拖曳的筆記、PDF 摘錄或圖片卡。".into());
            }
            if drop_target_bounds(self.board_area(), |value| self.scaled(value)).is_none() {
                return Err("視窗寬度不足，請放大視窗後重試。".into());
            }
            let boards: Vec<_> = self.notebook.list_boards()?.into_iter()
                .filter(|board| board.id != source).collect();
            if boards.is_empty() {
                label(self.widgets.status, "請先建立另一張目的白板。");
                return Ok(());
            }
            let choices: Vec<_> = boards.iter().enumerate()
                .map(|(index, board)| (index + 1, board.title.as_str())).collect();
            let chosen = popup(self.hwnd, self.widgets.links, None, &choices);
            if let Some(board) = chosen.checked_sub(1).and_then(|index| boards.get(index)) {
                self.extras.board_drop_target = Some(BoardDropTarget {
                    source, destination: board.id, title: board.title.clone(),
                });
                label(self.widgets.status, "拖曳選取的卡片到上方的目的白板，或按 Esc 取消。");
                InvalidateRect(self.hwnd, null(), 1);
            }
            Ok(())
        }
    }

    unsafe fn confirm_board_card_transfer(&mut self, target: &BoardDropTarget, expected: &[BoardPosition]) -> Result<bool> {
        unsafe {
            let Some(value) = crate::dialog::prompt(self.hwnd, "跨白板搬移",
                "輸入水平、垂直位移（dx,dy，可為負數）", "0,0", 32)? else { return Ok(false) };
            let (dx, dy) = value.split_once(',').ok_or("請用逗號分隔兩個整數。")?;
            let dx = dx.trim().parse::<i32>().map_err(|_| "水平位移須為整數。")?;
            let dy = dy.trim().parse::<i32>().map_err(|_| "垂直位移須為整數。")?;
            let question = format!("將 {} 張卡片搬到「{}」？\n位移（{dx}, {dy}）。原白板會移除這批位置。完整分區與所選卡片之間的連線也會搬移。",
                expected.len(), target.title);
            if message(self.hwnd, "搬移白板卡片", &question, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                return Ok(false);
            }
            let moved = match self.notebook.move_cards_between_boards(target.source, target.destination, expected, dx, dy) {
                Ok(moved) => moved,
                Err(error) => {
                    let _ = self.refresh_board();
                    return Err(error);
                }
            };
            let status = format!("已將 {moved} 張卡片搬到「{}」。", target.title);
            if let Err(error) = self.refresh_board() {
                label(self.widgets.status, &format!("{status}目前無法重新整理原白板。"));
                self.show_error(error);
            } else {
                label(self.widgets.status, &status);
            }
            Ok(true)
        }
    }

    pub(super) unsafe fn paint_board_drop_target(&self, dc: HDC, area: RECT) {
        unsafe {
            let Some(target) = self.extras.board_drop_target.as_ref().filter(|target| {
                target.source == self.board_id() && self.extras.workspace.is_none()
            }) else { return };
            let Some(rect) = drop_target_bounds(area, |value| self.scaled(value)) else { return };
            let hover = self.extras.board_batch_drag.as_ref().and_then(|drag| drag.drop_hover) == Some(true);
            fill(dc, &rect, if hover { TEAL } else { MINT });
            let brush = CreateSolidBrush(TEAL);
            if !brush.is_null() {
                FrameRect(dc, &rect, brush);
                DeleteObject(brush);
            }
            let prefix = if hover { "放開以搬至" } else { "拖到這裡搬至" };
            let mut caption = RECT { left: rect.left + self.scaled(8), right: rect.right - self.scaled(8), ..rect };
            draw_text(dc, self.fonts[4], if hover { PAPER } else { TEAL },
                &format!("{prefix}「{}」", target.title), &mut caption, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
        }
    }

    unsafe fn board_drop_input(&mut self, msg: u32, area: RECT, x: i32, y: i32, mut drag: BoardBatchDrag) {
        unsafe {
            match msg {
                WM_MOUSEMOVE => {
                    let inside = drop_target_bounds(area, |value| self.scaled(value))
                        .is_some_and(|rect| over_drop_target(rect, x, y));
                    if drag.drop_hover != Some(inside) {
                        drag.drop_hover = Some(inside);
                        InvalidateRect(self.hwnd, null(), 1);
                    }
                    self.extras.board_batch_drag = Some(drag);
                }
                WM_LBUTTONUP => {
                    let inside = drop_target_bounds(area, |value| self.scaled(value))
                        .is_some_and(|rect| over_drop_target(rect, x, y));
                    let target = self.extras.board_drop_target.take()
                        .filter(|target| inside && target.source == drag.board && drag.board == self.board_id());
                    if GetCapture() == self.hwnd { ReleaseCapture(); }
                    InvalidateRect(self.hwnd, null(), 1);
                    if let Some(target) = target {
                        label(self.widgets.status, "請確認跨板搬移的位移。");
                        match self.confirm_board_card_transfer(&target, &drag.expected) {
                            Ok(true) => {}
                            Ok(false) => label(self.widgets.status, "已取消跨板拖曳。"),
                            Err(error) => {
                                label(self.widgets.status, "未搬移跨板選取的卡片。");
                                self.show_error(error);
                            }
                        }
                    } else {
                        label(self.widgets.status, "已取消跨板拖曳。");
                    }
                }
                WM_CAPTURECHANGED | WM_CANCELMODE => {
                    self.extras.board_drop_target = None;
                    if GetCapture() == self.hwnd { ReleaseCapture(); }
                    InvalidateRect(self.hwnd, null(), 1);
                    label(self.widgets.status, "已取消跨板拖曳。");
                }
                _ => self.extras.board_batch_drag = Some(drag),
            }
        }
    }

    pub(super) fn board_card_collapsed(&self, note: i64) -> bool {
        self.extras.groups.iter().any(|group| group.members.contains(&note)
            && self.extras.collapsed_board_groups.contains(&(self.board_id(), group.id)))
    }

    pub(super) unsafe fn collapse_board_group_menu(&mut self) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let groups = &self.extras.groups;
            if groups.is_empty() {
                label(self.widgets.status, "這張白板沒有分區。");
                return Ok(());
            }
            let labels = groups.iter().map(|group| {
                let action = if self.extras.collapsed_board_groups.contains(&(board, group.id)) { "展開" } else { "收合" };
                format!("{action}「{}」", group.title)
            }).collect::<Vec<_>>();
            let choices = labels.iter().enumerate().map(|(i, text)| (i + 1, text.as_str())).collect::<Vec<_>>();
            let action = popup(self.hwnd, self.widgets.links, None, &choices);
            if let Some(group) = action.checked_sub(1).and_then(|index| groups.get(index)) {
                let key = (board, group.id);
                if self.extras.collapsed_board_groups.insert(key) {
                    if self.focused_native_card().is_some_and(|note| group.members.contains(&note)) {
                        self.extras.native_board_focus = None;
                    }
                    for note in &group.members {
                        self.extras.board_selected.remove(&BoardItem::Card(*note));
                    }
                    label(self.widgets.status, "已收合分區。卡片位置和連線仍保留。");
                } else {
                    self.extras.collapsed_board_groups.remove(&key);
                    label(self.widgets.status, "已展開分區。");
                }
                self.refresh_board_image_cache();
                InvalidateRect(self.hwnd, &self.board_area(), 1);
            }
            Ok(())
        }
    }

    pub(super) unsafe fn paint_board_item_frame(&self, dc: HDC, rect: RECT, item: BoardItem) {
        unsafe {
            let selected = self.extras.board_selected.contains(&item)
                || matches!(item, BoardItem::Card(id) if self.focused_native_card() == Some(id));
            let brush = CreateSolidBrush(if selected { TEAL } else { LINE });
            if !brush.is_null() {
                FrameRect(dc, &rect, brush);
                if selected {
                    let inset = self.scaled(3).max(1);
                    FrameRect(dc, &RECT { left: rect.left + inset, top: rect.top + inset,
                        right: rect.right - inset, bottom: rect.bottom - inset }, brush);
                }
                DeleteObject(brush);
            }
        }
    }

    pub(super) unsafe fn board_batch_input(&mut self, msg: u32, wp: WPARAM, x: i32, y: i32) -> bool {
        unsafe {
            let area = self.board_area();
            if let Some(mut drag) = self.extras.board_batch_drag.take() {
                if drag.drop_hover.is_some() {
                    self.board_drop_input(msg, area, x, y, drag);
                    return true;
                }
                match msg {
                    WM_MOUSEMOVE | WM_LBUTTONUP => {
                        let dpi = self.dpi as i32;
                        let visible_width = ((area.right - area.left) * 96 / dpi).max(0);
                        let min_x = drag.expected.iter().map(|entry| entry.x).min().unwrap_or(0);
                        let min_y = drag.expected.iter().map(|entry| entry.y).min().unwrap_or(0);
                        let right = drag.expected.iter().map(|entry| entry.x).max().unwrap_or(0);
                        let edge = drag.expected.iter().map(|entry| {
                            let width = match entry.item {
                                BoardItem::Card(id) => self.extras.cards.iter().find(|card| card.id == id)
                                    .map_or(232, |card| card.size().width),
                                BoardItem::Object(_) => 232,
                            };
                            entry.x + width
                        }).max().unwrap_or(0);
                        let max_dx = (self.extras.board_scroll_x + visible_width - edge).min(10000 - right).max(0);
                        let bottom = drag.expected.iter().map(|entry| entry.y).max().unwrap_or(0);
                        drag.delta = (((x - drag.start.0) * 96 / dpi).clamp(-min_x, max_dx),
                            ((y - drag.start.1) * 96 / dpi).clamp(-min_y, 50000 - bottom));
                        if msg == WM_MOUSEMOVE {
                            self.preview_board_delta(&drag.expected, drag.delta);
                            self.extras.board_batch_drag = Some(drag);
                            InvalidateRect(self.hwnd, &area, 1);
                        } else {
                            self.preview_board_delta(&drag.expected, (0, 0));
                            ReleaseCapture();
                            if drag.delta != (0, 0)
                                && let Err(error) = self.finish_board_selection_move(drag.board, &drag.expected, drag.delta)
                            {
                                self.show_error(error);
                            }
                            InvalidateRect(self.hwnd, &area, 1);
                        }
                    }
                    WM_CAPTURECHANGED | WM_CANCELMODE => {
                        self.preview_board_delta(&drag.expected, (0, 0));
                        if GetCapture() == self.hwnd { ReleaseCapture(); }
                        InvalidateRect(self.hwnd, &area, 1);
                    }
                    _ => { self.extras.board_batch_drag = Some(drag); }
                }
                return true;
            }
            if !matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK)
                || !(area.left..area.right).contains(&x) || !(area.top..area.bottom).contains(&y)
            {
                return false;
            }
            let card = self.card_at(x, y);
            let hit = card.filter(|id| self.focused_native_card() == Some(*id)).map(BoardItem::Card)
                .or_else(|| self.board_object_at(area, x, y).map(|object| BoardItem::Object(object.id)))
                .or_else(|| card.map(BoardItem::Card));
            const MOUSE_CONTROL: WPARAM = 0x0008; // MK_CONTROL in Win32 key-state flags.
            if wp & MOUSE_CONTROL != 0 {
                if let Some(item) = hit
                    && !self.extras.board_selected.insert(item)
                {
                    self.extras.board_selected.remove(&item);
                }
                if self.extras.board_drop_target.is_some() && (self.extras.board_selected.is_empty()
                    || self.extras.board_selected.iter().any(|item| matches!(item, BoardItem::Object(_))))
                {
                    self.extras.board_drop_target = None;
                    InvalidateRect(self.hwnd, null(), 1);
                }
                self.update_board_selection_status();
                InvalidateRect(self.hwnd, &area, 1);
                return true;
            }
            if msg == WM_LBUTTONDOWN && hit.is_some_and(|item| self.extras.board_selected.contains(&item)) {
                let drop_hover = self.extras.board_drop_target.as_ref()
                    .filter(|target| target.source == self.board_id()).map(|_| false);
                if self.extras.board_drop_target.is_some() && drop_hover.is_none() {
                    self.cancel_board_selection();
                    return true;
                }
                SetCapture(self.hwnd);
                if GetCapture() == self.hwnd {
                    self.extras.board_batch_drag = Some(BoardBatchDrag { board: self.board_id(), start: (x, y),
                        expected: self.board_selection_snapshot(), delta: (0, 0), drop_hover });
                } else if drop_hover.is_some() {
                    self.cancel_board_selection();
                }
                return true;
            }
            if self.extras.board_drop_target.is_some() {
                self.cancel_board_selection();
                return true;
            }
            if !self.extras.board_selected.is_empty() {
                self.extras.board_selected.clear();
                self.update_board_selection_status();
                InvalidateRect(self.hwnd, &area, 1);
            }
            false
        }
    }

    pub(super) unsafe fn cancel_board_selection(&mut self) -> bool {
        unsafe {
            let armed = self.extras.board_drop_target.take().is_some();
            if !armed && (!self.extras.board || self.extras.workspace.is_some()) { return false }
            let drag = self.extras.board_batch_drag.take();
            if let Some(drag) = drag {
                if drag.drop_hover.is_none() {
                    self.preview_board_delta(&drag.expected, (0, 0));
                }
                if GetCapture() == self.hwnd { ReleaseCapture(); }
            } else if !armed {
                if self.extras.board_selected.is_empty() { return false }
                self.extras.board_selected.clear();
            }
            if armed {
                label(self.widgets.status, "已取消跨板拖曳。");
                InvalidateRect(self.hwnd, null(), 1);
            } else {
                self.update_board_selection_status();
                InvalidateRect(self.hwnd, &self.board_area(), 1);
            }
            true
        }
    }

    unsafe fn update_board_selection_status(&self) {
        unsafe {
            let count = self.extras.board_selected.len();
            let text = if self.extras.board_drop_target.is_some() {
                "拖曳選取的卡片到上方的目的白板，或按 Esc 取消。".to_owned()
            } else if count > 0 { format!("已選取 {count} 個物件。拖曳或從白板操作指定整批位移。") }
                else { "按住 Ctrl 點選多個物件。".to_owned() };
            label(self.widgets.status, &text);
        }
    }

    fn preview_board_delta(&mut self, expected: &[BoardPosition], (dx, dy): (i32, i32)) {
        for entry in expected {
            let position = (entry.x + dx, entry.y + dy);
            match entry.item {
                BoardItem::Card(id) => if let Some(card) = self.extras.cards.iter_mut().find(|card| card.id == id) {
                    (card.x, card.y) = position;
                },
                BoardItem::Object(id) => if let Some(object) = self.extras.objects.iter_mut().find(|object| object.id == id) {
                    (object.x, object.y) = position;
                },
            }
        }
    }

    unsafe fn finish_board_selection_move(&mut self, board: i64, expected: &[BoardPosition], (dx, dy): (i32, i32)) -> Result<()> {
        unsafe {
            let moved = self.notebook.move_board_selection(board, expected, dx, dy);
            let refreshed = self.refresh_board();
            moved?;
            refreshed?;
            if (dx, dy) == (0, 0) {
                self.update_board_selection_status();
            } else {
                label(self.widgets.status, &format!("已移動 {} 個白板物件。", expected.len()));
            }
            Ok(())
        }
    }

    pub(super) fn board_path_label(&self) -> String {
        match self.notebook.board_path(self.board_id()) {
            Ok(path) => path.iter().map(|board| board.title.as_str()).collect::<Vec<_>>().join(" › "),
            Err(_) => self.extras.board_selection.as_ref()
                .map_or_else(|| "卡片白板".to_owned(), |(_, title)| title.clone()),
        }
    }

    pub(super) unsafe fn board_object_menu(&mut self)->Result<()> {
        unsafe {
            let board=self.board_id();
            let anchor=self.widgets.links;
            match popup(self.hwnd,anchor,None,&[(1,"新增子白板…"),(2,"開啟子白板…"),
                (3,"返回上層並定位子白板卡片"),(4,"變更目前白板的上層…"),(5,"新增文字方塊…"),(6,"編輯文字方塊…"),
                (7,"移除文字方塊…"),(8,"白板路徑…"),(9,"復原最近移除的文字方塊…"),(10,"將選取的卡片收進新子白板…"),
                (11,"將子白板卡片展回上層…")]) {
                1=>{
                    if let Some(title)=crate::dialog::prompt(self.hwnd,"新增子白板","白板名稱","",MAX_TITLE_CHARS)? {
                        let child=self.notebook.new_child_board(board,&title)?;
                        self.select_board(child,title.trim().to_owned())?;
                    }
                }
                action @ (2|6|7)=>{
                    let objects=self.notebook.objects_on(board)?;
                    let nodes=self.notebook.mind_nodes_on(board)?;
                    let list=objects.iter().filter(|o|o.child.is_some()==(action==2)
                        && !nodes.iter().any(|node|node.id==o.id)).collect::<Vec<_>>();
                    if list.is_empty(){
                        label(self.widgets.status,if action==2 {"請先新增子白板。"} else {"請先新增文字方塊。"});
                        return Ok(());
                    }
                    let labels=list.iter().map(|o|format!("{}  (#{})",o.text.lines().next().unwrap_or("").chars().take(60).collect::<String>(),o.id)).collect::<Vec<_>>();
                    let choices=labels.iter().enumerate().map(|(i,s)|(i+1,s.as_str())).collect::<Vec<_>>();
                    if let Some(o)=popup(self.hwnd,anchor,None,&choices).checked_sub(1).and_then(|i|list.get(i)){
                        if action == 7 {
                            self.confirm_remove_board_text(board, o)?;
                        } else {
                            self.open_board_object(o)?;
                        }
                    }
                }
                3=>{
                    self.return_to_parent_board_tile(board)?;
                }
                4=>{
                    if board==DEFAULT_BOARD_ID{return Err("預設白板位於最上層。".into());}
                    let previous=self.notebook.parent_board(board)?;
                    let boards=self.notebook.list_boards()?.into_iter().filter(|b|b.id!=board).collect::<Vec<_>>();
                    let mut choices=vec![(1,"移到最上層")];
                    choices.extend(boards.iter().enumerate().map(|(i,b)|(i+2,b.title.as_str())));
                    let target=match popup(self.hwnd,anchor,None,&choices) {
                        0=>return Ok(()),1=>None,n=>Some(boards[n-2].id),
                    };
                    self.notebook.reparent_board(board,previous,target)?;
                    self.refresh_board()?;
                    label(self.widgets.crumb, &self.board_path_label());
                    label(self.widgets.status,"白板的上層位置已儲存。");
                }
                5=>{
                    if let Some(text)=crate::dialog::prompt_multiline(self.hwnd,"新增文字方塊","文字只保存在目前白板；可拖曳移動，按兩下編輯。","",TEXT_LIMIT)? {
                        self.notebook.add_board_text(board,&text)?;
                        self.refresh_board()?;
                    }
                }
                8 => {
                    let mut path = self.notebook.board_path(board)?;
                    path.pop(); // Only ancestors are navigation targets.
                    if path.is_empty() {
                        label(self.widgets.status, "目前白板已在最上層。");
                        return Ok(());
                    }
                    let labels = path.iter().enumerate().map(|(depth, parent)| {
                        format!("{}{}", "　".repeat(depth), parent.title)
                    }).collect::<Vec<_>>();
                    let choices = labels.iter().enumerate()
                        .map(|(index, text)| (index + 1, text.as_str())).collect::<Vec<_>>();
                    if let Some(parent) = popup(self.hwnd, anchor, None, &choices)
                        .checked_sub(1).and_then(|index| path.get(index))
                    {
                        let current = self.notebook.board_path(board)?;
                        let (_, ancestors) = current.split_last().ok_or("找不到白板路徑。")?;
                        if !ancestors.iter().any(|item| item.id == parent.id) {
                            return Err("白板路徑已變更，請重新開啟選單。".into());
                        }
                        self.select_board(parent.id, parent.title.clone())?;
                    }
                }
                11 => {
                    let plan = self.notebook.plan_child_unfold(board)?;
                    let question = format!("將 {} 張卡片依目前座標展回上層？完整分區與連線會一起搬移。子白板及其文字和分支會保留，入口若與卡片重疊會移到空位。",plan.count());
                    if message(self.hwnd,"確認展回上層",&question,MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2)!=IDYES { return Ok(()); }
                    self.notebook.unfold_child(&plan)?;
                    self.return_to_parent_board_tile(board)?;
                    label(self.widgets.status,&format!("已展回 {} 張卡片，並定位原子白板入口。",plan.count()));
                }
                10 => {
                    let selected=self.board_selection_snapshot();
                    if selected.is_empty(){label(self.widgets.status,"請按住 Ctrl 點選要收進子白板的卡片。");return Ok(());}
                    let plan=self.notebook.plan_selection_child(board,&selected)?;
                    let Some(title)=crate::dialog::prompt(self.hwnd,"將卡片收進新子白板","新子白板名稱","",MAX_TITLE_CHARS)? else{return Ok(());};
                    let question=format!("將 {} 張卡片及其連線與完整分區搬到「{}」？原白板會在空位放置子白板入口。",selected.len(),title.trim());
                    if message(self.hwnd,"確認收進子白板",&question,MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2)!=IDYES{return Ok(());}
                    let child=self.notebook.commit_selection_child(&plan,&title)?;
                    self.return_to_parent_board_tile(child)?;
                    label(self.widgets.status,&format!("已將 {} 張卡片收進「{}」，並定位子白板入口。",selected.len(),title.trim()));
                }
                9 => {
                    let Some(removed) = self.notebook.latest_removed_board_text(board)? else {
                        label(self.widgets.status, "目前白板沒有可復原的文字方塊。");
                        return Ok(());
                    };
                    let preview: String = removed.text.chars().take(120).collect();
                    let suffix = if removed.text.chars().count() > 120 { "…" } else { "" };
                    let question = format!("復原最近移除的文字方塊？\n\n{preview}{suffix}");
                    if message(self.hwnd, "復原文字方塊", &question,
                               MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES {
                        let exact = self.notebook.restore_board_text(board, &removed)?;
                        self.refresh_board()?;
                        label(self.widgets.status, if exact { "文字方塊已復原到原位置。" }
                            else { "原位置已被占用，文字方塊已復原到空位。" });
                    }
                }
                _=>{}
            }
            Ok(())
        }
    }

    unsafe fn confirm_remove_board_text(&mut self, board: i64, object: &Object) -> Result<()> {
        unsafe {
            let previous = self.notebook.latest_removed_board_text(board)?;
            let question = if previous.is_some() {
                "移除選取的文字方塊？這會取代此白板上一筆可復原的文字方塊。"
            } else {
                "移除選取的文字方塊？此白板會保留這一筆供復原。"
            };
            if message(self.hwnd, "移除文字方塊", question,
                       MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) == IDYES {
                self.notebook.remove_board_text(board, object, previous.as_ref())?;
                self.refresh_board()?;
                label(self.widgets.status, "已移除。可在「子白板與文字」復原此白板最近移除的方塊。");
            }
            Ok(())
        }
    }

    pub(super) unsafe fn create_child_board_from_card_ui(
        &mut self, note: i64, expected: (i32, i32),
    ) -> Result<()> {
        unsafe {
            let Some(title) = crate::dialog::prompt(
                self.hwnd, "從此卡建立子白板",
                "白板名稱。子白板會放入同一張筆記卡，原白板的位置保留。", "", MAX_TITLE_CHARS,
            )? else { return Ok(()); };
            let child = self.notebook.new_child_board_from_card(self.board_id(), note, expected, &title)?;
            self.select_board(child, title.trim().to_owned())?;
            self.extras.board_selected.insert(BoardItem::Card(note));
            self.present();
            label(self.widgets.status, "已建立子白板並放入同一張筆記卡。原卡片位置仍保留。");
            Ok(())
        }
    }

    unsafe fn return_to_parent_board_tile(&mut self, child: i64) -> Result<()> {
        use crate::store::native_board_locations::reveal_offset;
        unsafe {
            let Some((parent, tile)) = self.notebook.parent_board_tile(child)? else {
                label(self.widgets.status, "目前已在最上層。");
                return Ok(());
            };
            self.select_board(parent.id, parent.title)?;
            let object = self.extras.objects.iter().find(|object| object.id == tile && object.child == Some(child))
                .ok_or("子白板已移動，請重新開啟白板。")?;
            let (x, y) = (object.x, object.y);
            let area = self.board_area();
            let width = (area.right - area.left).max(0) * 96 / self.dpi as i32;
            let height = (area.bottom - area.top).max(0) * 96 / self.dpi as i32;
            self.extras.board_scroll_x = reveal_offset(x, ImageSize::CARD.width, width);
            self.extras.scroll = reveal_offset(y, ImageSize::CARD.height, height).min(self.board_scroll_end());
            self.extras.board_selected.insert(BoardItem::Object(tile));
            self.refresh_board_image_cache();
            self.present();
            label(self.widgets.status, "已定位上層的子白板卡片。");
            Ok(())
        }
    }

    unsafe fn open_board_object(&mut self,object:&Object)->Result<()> {
        unsafe {
            if let Some(id) = object.child {
                let child = self.notebook.child_board_from_tile(self.board_id(), object.id, id)?;
                self.select_board(child.id, child.title)?;
            }
            else if let Some(text)=crate::dialog::prompt_multiline(self.hwnd,"編輯文字方塊","白板文字",&object.text,TEXT_LIMIT)? {
                self.notebook.edit_board_text(self.board_id(),object,&text)?;
                self.refresh_board()?;
            }
            Ok(())
        }
    }

    fn object_rect(&self,area:RECT,object:&Object)->RECT {
        let left=self.board_screen_x(area, object.x);
        let top=area.top+self.scaled(object.y-self.extras.scroll);
        RECT{left,top,right:left+self.scaled(232),bottom:top+self.scaled(114)}
    }

    fn board_object_at(&self, area: RECT, x: i32, y: i32) -> Option<&Object> {
        self.extras.objects.iter().rev().find(|o| {
            let r = self.object_rect(area, o);
            (r.left..r.right).contains(&x) && (r.top..r.bottom).contains(&y)
        })
    }

    pub(super) unsafe fn paint_board_objects(&self,dc:HDC,area:RECT) {
        unsafe {
            for o in &self.extras.objects{
                let rect=self.object_rect(area,o);
                if rect.bottom<=area.top || rect.top>=area.bottom{continue;}
                if let Some(node)=self.extras.mind_map_nodes.iter().find(|node|node.id==o.id){
                    self.paint_mind_map_node(dc, rect, node);
                    continue;
                }
                let child=o.child.is_some();
                fill(dc,&rect,if child{color(229,241,237)}else{color(254,249,222)});
                self.paint_board_item_frame(dc, rect, BoardItem::Object(o.id));
                let mut heading=RECT{left:rect.left+self.scaled(12),top:rect.top+self.scaled(8),right:rect.right-self.scaled(12),bottom:rect.top+self.scaled(32)};
                draw_text(dc,self.fonts[3],TEAL,if child{"子白板"}else{"文字方塊"},&mut heading,DT_SINGLELINE|DT_END_ELLIPSIS);
                let mut content=RECT{top:rect.top+self.scaled(34),bottom:rect.bottom-self.scaled(8),..heading};
                draw_text(dc,self.fonts[0],SOFT,&o.text,&mut content,DT_WORDBREAK|DT_END_ELLIPSIS|DT_NOPREFIX);
            }
        }
    }

    pub(super) unsafe fn board_object_input(&mut self,msg:u32,x:i32,y:i32)->bool {
        unsafe {
            if self.extras.dragging.is_some(){return false;}
            let board=self.board_id();
            let area=self.board_area();
            if let Some((id,dx,dy))=self.extras.object_drag {
                match msg {
                    WM_MOUSEMOVE=>{
                        let dpi=self.dpi as i32;
                        let scroll_x = self.extras.board_scroll_x;
                        let max_x=(scroll_x+(area.right-area.left)*96/dpi-232).clamp(0,10000);
                        let left=((x-area.left-dx)*96/dpi+scroll_x).clamp(0,max_x);
                        let top=((y-area.top-dy)*96/dpi+self.extras.scroll).clamp(0,50000);
                        if let Some(o)=self.extras.objects.iter_mut().find(|o|o.id==id){o.x=left;o.y=top;}
                        InvalidateRect(self.hwnd,&area,1);
                    }
                    WM_LBUTTONUP=>{
                        self.extras.object_drag=None;
                        ReleaseCapture();
                        if let Some(o)=self.extras.objects.iter().find(|o|o.id==id) {
                            let moved = if let Some(node) = self.extras.mind_map_nodes.iter().find(|node|node.id==id) {
                                if (node.x, node.y) == (o.x, o.y) {
                                    Ok(())
                                } else {
                                    let expected = BoardPosition {
                                        item: BoardItem::Object(id), x: node.x, y: node.y, version: Some(node.version),
                                    };
                                    self.notebook.move_board_selection(board, &[expected], o.x - node.x, o.y - node.y)
                                }
                            } else {
                                self.notebook.move_board_object(board,o)
                            };
                            if let Err(error) = moved { self.show_error(error); }
                        }
                        if let Err(e)=self.refresh_board(){self.show_error(e);}
                    }
                    WM_CAPTURECHANGED=>{
                        self.extras.object_drag=None;
                        if let Err(e)=self.refresh_board(){self.show_error(e);}
                    }
                    _=>return false,
                }
                return true;
            }
            if !matches!(msg,WM_LBUTTONDOWN|WM_LBUTTONDBLCLK|WM_RBUTTONUP)
                || x<area.left || y<area.top || x>=area.right || y>=area.bottom{return false;}
            let Some(o)=self.board_object_at(area, x, y).cloned() else{return false;};
            if msg==WM_LBUTTONDOWN{
                let rect=self.object_rect(area,&o);
                self.extras.object_drag=Some((o.id,x-rect.left,y-rect.top));
                SetCapture(self.hwnd);
            } else {
                let result = if let Some(node) = self.extras.mind_map_nodes.iter().find(|node|node.id==o.id).cloned() {
                    if msg == WM_RBUTTONUP {
                        let mut point = POINT { x, y };
                        ClientToScreen(self.hwnd, &mut point);
                        self.mind_node_menu(&node, Some(point))
                    } else {
                        self.edit_mind_node_ui(&node)
                    }
                } else if msg == WM_RBUTTONUP && o.child.is_some() {
                    let mut point = POINT { x, y };
                    ClientToScreen(self.hwnd, &mut point);
                    if popup(self.hwnd, self.widgets.links, Some(point), &[(1, "開啟子白板")]) == 1 {
                        self.open_board_object(&o)
                    } else {
                        Ok(())
                    }
                } else if msg == WM_RBUTTONUP && o.child.is_none() {
                    let mut point = POINT { x, y };
                    ClientToScreen(self.hwnd, &mut point);
                    match popup(self.hwnd, self.widgets.links, Some(point), &[
                        (1, "編輯文字方塊…"), (2, "移除文字方塊…"),
                    ]) {
                        1 => self.open_board_object(&o),
                        2 => self.confirm_remove_board_text(board, &o),
                        _ => Ok(()),
                    }
                } else {
                    self.open_board_object(&o)
                };
                if let Err(error) = result {
                    self.show_error(error);
                }
            }
            true
        }
    }
}

#[cfg(test)]
mod drop_target_tests {
    use super::*;

    #[test]
    fn header_drop_target_has_exact_dpi_scaled_hit_bounds() {
        for dpi in [96, 144] {
            let scale = |v| (v * dpi + 48) / 96;
            let area = RECT { left: scale(330), right: scale(1173), top: scale(100), bottom: scale(650) };
            let shelf = drop_target_bounds(area, scale).unwrap();
            assert_eq!((shelf.left, shelf.right, shelf.top, shelf.bottom),
                (scale(450), scale(770), scale(62), scale(94)));
            assert!(over_drop_target(shelf, shelf.left, shelf.top));
            assert!(over_drop_target(shelf, shelf.right - 1, shelf.bottom - 1));
            assert!(!over_drop_target(shelf, shelf.right, shelf.top));
            assert!(!over_drop_target(shelf, shelf.left, shelf.bottom));
            let narrow = RECT { right: scale(580), ..area };
            assert!(drop_target_bounds(narrow, scale).is_none());
        }
    }
}
