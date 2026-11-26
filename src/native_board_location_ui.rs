use super::*;
use crate::store::native_board_locations::reveal_offset;

impl App {
    pub(super) unsafe fn show_native_board_locations(&mut self, note: i64) -> Result<()> {
        let locations = self.notebook.native_board_locations(note)?;
        if locations.is_empty() {
            unsafe { label(self.widgets.status, "這篇筆記尚未放入原生白板。"); }
            return Ok(());
        }
        let Some(index) = (unsafe {
            navigation_ui::choose_navigation_page(self.hwnd, self.widgets.links, locations.len(), |i| {
                &locations[i].title
            })
        }) else { return Ok(()); };
        let location = locations.into_iter().nth(index).ok_or("找不到選取的白板。")?;
        self.notebook.note_at_native_board_location(&location)?;
        unsafe {
            self.select_board(location.board, location.title.clone())?;
            let card = self.extras.cards.iter().find(|card| card.id == note)
                .ok_or("卡片已移出白板，請重新查看白板位置。")?;
            let size = card.size();
            let (x, y) = (card.x, card.y);
            let area = self.board_area();
            let width = (area.right - area.left).max(0) * 96 / self.dpi as i32;
            let height = (area.bottom - area.top).max(0) * 96 / self.dpi as i32;
            self.extras.board_scroll_x = reveal_offset(x, size.width, width);
            self.extras.scroll = reveal_offset(y, size.height, height).min(self.board_scroll_end());
            self.extras.collapsed_board_groups.retain(|(board, id)| {
                *board != location.board
                    || !self.extras.groups.iter().any(|group| group.id == *id && group.members.contains(&note))
            });
            self.extras.native_board_focus = Some(location);
            self.extras.board = true;
            self.extras.workspace = None;
            self.refresh_board_image_cache();
            self.present();
            label(self.widgets.status, "已定位白板上的筆記卡片。");
        }
        Ok(())
    }

    pub(super) fn focused_native_card(&self) -> Option<i64> {
        self.extras.native_board_focus.as_ref().map(|location| location.note)
    }

    pub(super) fn board_screen_x(&self, area: RECT, x: i32) -> i32 {
        area.left + self.scaled(x - self.extras.board_scroll_x)
    }
}

