use super::board_shape_input::parse_shape_geometry;
use super::navigation_ui::choose_navigation;
use super::*;
use crate::store::board_shapes::{Shape, ShapeKind, ShapeRect};
use crate::store::board_tags::TagSubject;
use crate::store::vault_search::TagMatch;

fn shape_label(kind: ShapeKind) -> &'static str {
    match kind { ShapeKind::Rectangle => "矩形", ShapeKind::Ellipse => "橢圓" }
}

impl App {
    unsafe fn prompt_shape_rect(&self, title: &str, rect: ShapeRect) -> Result<Option<ShapeRect>> {
        unsafe {
            let mut prior = format!("{}, {}, {}, {}", rect.x, rect.y, rect.width, rect.height);
            let mut hint = "位置與尺寸：X、Y、寬、高，例如 96, 120, 160, 96".to_owned();
            loop {
                let Some(raw) = crate::dialog::prompt(self.hwnd, title, &hint, &prior, 80)? else {
                    return Ok(None);
                };
                match parse_shape_geometry(&raw) {
                    Ok([x,y,width,height]) => return Ok(Some(ShapeRect { x,y,width,height })),
                    Err(error) => { prior = raw; hint = error.to_owned(); }
                }
            }
        }
    }

    unsafe fn pick_board_shape(&self, shapes: &[Shape]) -> Option<Shape> {
        unsafe {
            if shapes.is_empty() {
                label(self.widgets.status, "這張白板還沒有形狀。");
                return None;
            }
            let entries = shapes.iter().map(|s| format!("{} #{}　{}, {}　{}×{}",
                shape_label(s.kind),s.id,s.rect.x,s.rect.y,s.rect.width,s.rect.height
            )).collect::<Vec<_>>();
            choose_navigation(self.hwnd, self.widgets.links, &entries).and_then(|index| shapes.get(index).copied())
        }
    }

    pub(super) unsafe fn board_shapes_menu(&mut self) -> Result<()> {
        unsafe {
            let board = self.board_id();
            match popup(self.hwnd, self.widgets.links, None, &[
                (1,"新增矩形…"), (2,"新增橢圓…"), (3,"移動或調整形狀…"), (4,"移除形狀…"),
            ]) {
                action @ (1 | 2) => {
                    let kind = if action == 1 { ShapeKind::Rectangle } else { ShapeKind::Ellipse };
                    let title = format!("新增{}",shape_label(kind));
                    let rect = ShapeRect {
                        x:32,y:(self.extras.scroll+48).clamp(0,50000),width:160,height:96,
                    };
                    if let Some(rect) = self.prompt_shape_rect(&title,rect)? {
                        self.notebook.create_board_shape(board,kind,rect)?;
                        self.refresh_board()?;
                        label(self.widgets.status,"已新增白板形狀。");
                    }
                }
                action @ (3 | 4) => {
                    let shapes = self.notebook.board_shapes(board)?;
                    if let Some(shape) = self.pick_board_shape(&shapes) {
                        if action == 3 {
                            self.edit_board_shape(board, shape)?;
                        } else {
                            self.confirm_remove_board_shape(board, shape)?;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }

    unsafe fn edit_board_shape(&mut self, board: i64, shape: Shape) -> Result<()> {
        unsafe {
            if let Some(rect) = self.prompt_shape_rect("移動或調整形狀", shape.rect)?
                && rect != shape.rect
            {
                self.notebook.update_board_shape(board, shape.id, shape.version, rect)?;
                self.refresh_board()?;
                label(self.widgets.status, "白板形狀的位置與尺寸已儲存。");
            }
            Ok(())
        }
    }

    unsafe fn confirm_remove_board_shape(&mut self, board: i64, shape: Shape) -> Result<()> {
        unsafe {
            let message = format!("移除{} #{}？", shape_label(shape.kind), shape.id);
            if MessageBoxW(self.hwnd, wide(&message).as_ptr(), wide("移除白板形狀").as_ptr(),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) == IDYES
            {
                self.notebook.delete_board_shape(board, shape.id, shape.version)?;
                self.refresh_board()?;
                label(self.widgets.status, "白板形狀已移除。");
            }
            Ok(())
        }
    }

    fn board_shape_pixel_rect(&self, shape: Shape, area: RECT) -> ShapeRect {
        ShapeRect {
            x: area.left + self.scaled(shape.rect.x),
            y: area.top + self.scaled(shape.rect.y - self.extras.scroll),
            width: self.scaled(shape.rect.width),
            height: self.scaled(shape.rect.height),
        }
    }

    pub(super) unsafe fn board_shape_at(&self, x: i32, y: i32) -> Option<Shape> {
        unsafe {
            let area = self.board_area();
            if x < area.left || x >= area.right || y < area.top || y >= area.bottom {
                return None;
            }
            // The last painted shape is topmost. Notes and text win before this fallback.
            self.extras.board_shapes.iter().rev().copied().find(|shape| {
                self.board_shape_pixel_rect(*shape, area).hit(shape.kind, x, y)
            })
        }
    }

    pub(super) unsafe fn board_shape_context_menu(&mut self, shape: Shape, point: POINT) -> Result<()> {
        unsafe {
            let board = self.board_id();
            match popup(self.hwnd, null_mut(), Some(point), &[
                (1, "移動或調整這個形狀…"),
                (2, "移除這個形狀…"),
            ]) {
                1 => self.edit_board_shape(board, shape),
                2 => self.confirm_remove_board_shape(board, shape),
                _ => Ok(()),
            }
        }
    }

    pub(super) unsafe fn paint_board_shapes(&self, dc: HDC, area: RECT) {
        unsafe {
            let pen = CreatePen(PS_SOLID,self.scaled(2).max(1),color(53,100,137));
            let brush = CreateSolidBrush(color(225,239,249));
            if !pen.is_null() && !brush.is_null() {
                let pen_before = SelectObject(dc,pen);
                let brush_before = SelectObject(dc,brush);
                for shape in &self.extras.board_shapes {
                    let ShapeRect { x, y, width, height } = self.board_shape_pixel_rect(*shape, area);
                    let right = x + width;
                    let bottom = y + height;
                    if x >= area.right || right <= area.left || y >= area.bottom || bottom <= area.top {continue;}
                    match shape.kind {
                        ShapeKind::Rectangle => { Rectangle(dc,x,y,right,bottom); }
                        ShapeKind::Ellipse => { Ellipse(dc,x,y,right,bottom); }
                    }
                }
                SelectObject(dc,brush_before);
                SelectObject(dc,pen_before);
            }
            if !brush.is_null() { DeleteObject(brush); }
            if !pen.is_null() { DeleteObject(pen); }
        }
    }

    pub(super) unsafe fn board_tags_menu(&mut self) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let tags = self.notebook.board_tags(board)?;
            match popup(self.hwnd, self.widgets.links, None, &[
                (1,"新增白板標籤…"), (2,"查看或移除目前白板標籤…"),
                (3,"按標籤找白板…"), (4,"按標籤找白板與某個 Vault 的筆記…"),
            ]) {
                1 => {
                    if let Some(tag) = crate::dialog::prompt(
                        self.hwnd,"新增白板標籤","例如 #觀察 或 #拍攝/夜景","",512,
                    )? {
                        self.notebook.set_board_tag(board,&tag,true)?;
                        label(self.widgets.status,"目前白板的標籤已儲存。");
                    }
                }
                2 => {
                    if tags.is_empty() {
                        label(self.widgets.status,"目前白板沒有標籤。");
                    } else {
                        let entries = tags.iter().map(|tag|format!("#{tag}")).collect::<Vec<_>>();
                        if let Some(index) = choose_navigation(self.hwnd,self.widgets.links,&entries)
                            && let Some(tag) = tags.get(index) {
                            let message = format!("只從目前白板移除 #{tag}？筆記的標籤不會變更。");
                            if MessageBoxW(self.hwnd,wide(&message).as_ptr(),wide("移除白板標籤").as_ptr(),
                                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) == IDYES {
                                self.notebook.set_board_tag(board,tag,false)?;
                                label(self.widgets.status,"已移除目前白板的標籤。");
                            }
                        }
                    }
                }
                3 => self.find_board_tags(None)?,
                4 => {
                    let vaults = self.notebook.imported_vaults()?;
                    if vaults.is_empty() {
                        label(self.widgets.status,"要一起搜尋筆記，請先匯入一個 Vault。");
                    } else {
                        let labels = vaults.iter().map(|(id,title)|format!("{title} (#{id})")).collect::<Vec<_>>();
                        if let Some(index) = choose_navigation(self.hwnd,self.widgets.links,&labels) {
                            self.find_board_tags(vaults.get(index).map(|(id,_)|*id))?;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }

    unsafe fn find_board_tags(&mut self, vault: Option<i64>) -> Result<()> {
        unsafe {
            let Some(tag) = crate::dialog::prompt(self.hwnd,"按完整標籤搜尋","例如 #觀察","",512)? else {
                return Ok(());
            };
            let hits = self.notebook.search_board_and_vault_tag(&tag, vault)?;
            if hits.is_empty() {
                label(self.widgets.status,"找不到此標籤。搜尋只包含白板與所選 Vault。");
                return Ok(());
            }
            let entries = hits.iter().map(|h| match h.subject {
                TagSubject::Board(id) => format!("白板　{} (#{id})",h.title),
                TagSubject::Note(id) => format!("筆記　{} (#{id})",h.title),
            }).collect::<Vec<_>>();
            if let Some(index) = choose_navigation(self.hwnd,self.widgets.links,&entries)
                && let Some(hit) = hits.get(index) {
                match hit.subject {
                    TagSubject::Board(id) => {
                        let Some(current) = self.notebook.search_board_and_vault_tag(&tag, None)?
                            .into_iter().find(|row|row.subject==hit.subject) else {
                            return Err("白板的標籤已變更，請重新搜尋。".into());
                        };
                        self.select_board(id,current.title)?;
                    }
                    TagSubject::Note(id) => {
                        let Some(root) = vault else { return Err("找不到筆記來源 Vault。".into()); };
                        if !self.notebook.vault_search_hit(root,id,&hit.title)?
                            || !self.notebook.search_vault_tag_terms(root,&[(tag,TagMatch::Exact)])?
                                .iter().any(|(matched_id,path)|*matched_id==id && *path==hit.title) {
                            return Err("Vault 來源或標籤已變更，請重新搜尋。".into());
                        }
                        self.open_import(id,"已開啟符合標籤的 Vault 筆記。");
                    }
                }
            }
            Ok(())
        }
    }
}
