//! Native board presentation and the two explicit image clipboard paths.
use super::*;
use crate::board_image_dib::{MAX_DIB_BYTES, MAX_DROP_BYTES, decode_clipboard_dib, decode_dropfiles};
use crate::store::board_images::{BoardImageInput, ImageSize, decode_image};
use crate::store::Card;
use image::ImageEncoder;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

// Native format identifiers from the clipboard API (Win32 System/Ole constants).
const DIB: u32 = 8;
const HDROP: u32 = 15;
const DIB_V5: u32 = 17;
const THUMB_SIDE: u32 = 384;
// Twenty worst-case BGRA previews use at most 11.25 MiB.
const MAX_CACHED_THUMBS: usize = 20;

struct Clipboard;

impl Clipboard {
    fn open(hwnd: HWND) -> Result<Self> {
        if unsafe { OpenClipboard(hwnd) } == 0 {
            return Err("剪貼簿目前無法開啟，請稍後再試。".into());
        }
        Ok(Self)
    }

    fn has(&self, format: u32) -> bool {
        unsafe { IsClipboardFormatAvailable(format) != 0 }
    }

    fn read<T>(&self, format: u32, limit: usize, decode: impl FnOnce(&[u8]) -> Result<T>) -> Result<T> {
        unsafe {
            let handle = GetClipboardData(format);
            if handle.is_null() { return Err("無法讀取該剪貼簿格式。".into()) }
            let len = GlobalSize(handle);
            if len == 0 || len > limit {
                return Err("剪貼簿影像或清單超出可處理的大小。".into());
            }
            let ptr = GlobalLock(handle).cast::<u8>();
            if ptr.is_null() { return Err("無法讀取剪貼簿記憶體。".into()) }
            let _lock = LockedGlobal(handle);
            decode(std::slice::from_raw_parts(ptr, len))
        }
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe { CloseClipboard(); }
    }
}

struct LockedGlobal(HGLOBAL);

impl Drop for LockedGlobal {
    fn drop(&mut self) {
        unsafe { GlobalUnlock(self.0); }
    }
}

fn clipboard_pixels(hwnd: HWND) -> Result<image::RgbaImage> {
    let clipboard = Clipboard::open(hwnd)?;
    let mut failure = None;
    for format in [DIB_V5, DIB] {
        if clipboard.has(format) {
            match clipboard.read(format, MAX_DIB_BYTES, |data| decode_clipboard_dib(data, format == DIB_V5)) {
                Ok(image) => return Ok(image),
                Err(error) => failure = Some(error),
            }
        }
    }
    Err(failure.unwrap_or_else(|| "剪貼簿內沒有可讀取的點陣圖片。請複製圖片本身後再貼上。".into()))
}

fn clipboard_files(hwnd: HWND) -> Result<Vec<PathBuf>> {
    let clipboard = Clipboard::open(hwnd)?;
    if !clipboard.has(HDROP) {
        return Err("剪貼簿內沒有圖片檔案清單。請從檔案總管複製 PNG 或 JPEG。".into());
    }
    clipboard.read(HDROP, MAX_DROP_BYTES, decode_dropfiles)?.into_iter().map(|text| {
        let path = PathBuf::from(text);
        if !path.is_absolute() {
            return Err("剪貼簿檔案須有完整的本機路徑。".into());
        }
        Ok(path)
    }).collect()
}

pub(super) struct BoardImageThumbnail {
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

fn fitted_image(width: i32, height: i32, max_width: i32, max_height: i32) -> (i32, i32) {
    if i64::from(max_width) * i64::from(height) <= i64::from(max_height) * i64::from(width) {
        (max_width, ((i64::from(max_width) * i64::from(height) / i64::from(width)) as i32).max(1))
    } else {
        (((i64::from(max_height) * i64::from(width) / i64::from(height)) as i32).max(1), max_height)
    }
}

fn thumbnail(data: &[u8]) -> Result<BoardImageThumbnail> {
    let image = decode_image(data)?;
    let image = if image.width().max(image.height()) > THUMB_SIDE {
        let (width, height) = fitted_image(image.width() as i32, image.height() as i32, THUMB_SIDE as i32, THUMB_SIDE as i32);
        image::imageops::thumbnail(&image, width as u32, height as u32)
    } else { image };
    let (width, height) = (image.width() as i32, image.height() as i32);
    let mut bgra = image.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for color in &mut pixel[..3] {
            *color = ((u16::from(*color) * alpha + 255 * (255 - alpha)) / 255) as u8;
        }
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Ok(BoardImageThumbnail { width, height, bgra })
}

fn thumbnail_distance(top: i32, bottom: i32, viewport_height: i32) -> Option<i32> {
    let distance = (-bottom).max(top - viewport_height).max(0);
    (distance <= viewport_height).then_some(distance)
}

impl App {
    pub(super) unsafe fn paste_board_image_pixels(&mut self) -> Result<()> {
        let image = clipboard_pixels(self.hwnd)?;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(
            image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgba8,
        )?;
        let entry = BoardImageInput::new("剪貼簿圖片".to_owned(), png)?;
        unsafe { self.add_board_images_ui(&[entry]) }
    }

    pub(super) unsafe fn paste_board_image_files(&mut self) -> Result<()> {
        let files = clipboard_files(self.hwnd)?;
        let entries = files.iter().map(|path| BoardImageInput::read(path)).collect::<Result<Vec<_>>>()?;
        unsafe { self.add_board_images_ui(&entries) }
    }

    pub(super) unsafe fn import_board_image_file(&mut self) -> Result<()> {
        unsafe {
            if let Some(file) = file_dialog(self.hwnd, "本機圖片\0*.png;*.jpg;*.jpeg\0\0", "", None)? {
                self.add_board_images_ui(&[BoardImageInput::read(&file)?])?;
            }
            Ok(())
        }
    }

    unsafe fn add_board_images_ui(&mut self, entries: &[BoardImageInput]) -> Result<()> {
        unsafe {
            let ids = self.notebook.add_board_images(self.board_id(), entries)?;
            let status = format!("已新增 {} 張本機圖片卡。", ids.len());
            if let Err(error) = self.refresh_board() {
                label(self.widgets.status, &format!("{status}目前無法重新整理白板。"));
                self.show_error(error);
            } else {
                label(self.widgets.status, &status);
            }
            Ok(())
        }
    }

    pub(super) unsafe fn resize_board_image_ui(&mut self, note: i64) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let card = self.extras.cards.iter().find(|card| card.id == note && card.image.is_some())
                .ok_or("找不到這張圖片卡，請重新整理白板。")?;
            let expected = card.size();
            let expected_at = (card.x, card.y);
            let Some(value) = crate::dialog::prompt(
                self.hwnd, "調整圖片卡大小", "輸入白板寬、高（64–768，使用逗號分隔）",
                &format!("{},{}", expected.width, expected.height), 16,
            )? else { return Ok(()) };
            let (width, height) = value.split_once(',').ok_or("請用逗號分隔寬度和高度。")?;
            let width = width.trim().parse::<i32>().map_err(|_| "寬度須為整數。")?;
            let height = height.trim().parse::<i32>().map_err(|_| "高度須為整數。")?;
            let next = ImageSize { width, height };
            next.validate()?;
            let area = self.board_area();
            let available = self.extras.board_scroll_x + (area.right - area.left) * 96 / self.dpi as i32 - expected_at.0;
            if width > expected.width && width > available {
                return Err("目前位置的可見寬度不足。請先把圖片卡移到左側或放大視窗。".into());
            }
            if next != expected {
                if let Err(error) = self.notebook.resize_board_image(board, note, expected_at, expected, next) {
                    let _ = self.refresh_board();
                    return Err(error);
                }
                let status = format!("圖片卡已調整為 {width} × {height}。");
                if let Err(error) = self.refresh_board() {
                    label(self.widgets.status, &format!("{status}目前無法重新整理白板。"));
                    self.show_error(error);
                } else {
                    label(self.widgets.status, &status);
                }
            }
            Ok(())
        }
    }

    pub(super) fn refresh_board_image_cache(&mut self) {
        if !self.extras.board || self.extras.workspace.is_some() {
            self.extras.board_image_thumbs.clear();
            return;
        }
        let area = unsafe { self.board_area() };
        let (width, height) = (area.right - area.left, area.bottom - area.top);
        if width <= 0 || height <= 0 {
            self.extras.board_image_thumbs.clear();
            return;
        }
        // Use the same client offsets and DPI transformation as board painting.
        // Nearby offscreen cards can be decoded before the next wheel step.
        let mut wanted = self.extras.cards.iter().filter_map(|card| {
            let size = card.image?;
            if self.board_card_collapsed(card.id) { return None }
            let left = self.board_screen_x(area, card.x) - area.left;
            if left >= width || left + self.scaled(size.width) <= 0 { return None }
            let top = self.scaled(card.y - self.extras.scroll);
            let bottom = top + self.scaled(size.height);
            thumbnail_distance(top, bottom, height)
                .map(|distance| (distance, std::cmp::Reverse(card.id)))
        }).collect::<Vec<_>>();
        wanted.sort_unstable();
        wanted.truncate(MAX_CACHED_THUMBS);
        let thumbnails = &mut self.extras.board_image_thumbs;
        thumbnails.retain(|id, _| wanted.iter().any(|(_, wanted)| wanted.0 == *id));
        for (_, std::cmp::Reverse(id)) in wanted {
            if thumbnails.contains_key(&id) { continue }
            if let Ok(bytes) = self.notebook.board_image_bytes(id) {
                thumbnails.insert(id, bytes.and_then(|bytes| thumbnail(&bytes).ok()));
            }
        }
    }

    pub(super) unsafe fn paint_board_image(&self, dc: HDC, rect: RECT, card: &Card) -> bool {
        unsafe {
            if card.image.is_none() { return false }
            let margin = self.scaled(8);
            let mut title = RECT { left: rect.left + margin, top: rect.top + self.scaled(5),
                right: rect.right - margin, bottom: rect.top + self.scaled(24) };
            draw_text(dc, self.fonts[3], TEAL, &card.title, &mut title, DT_SINGLELINE | DT_END_ELLIPSIS);
            let mut area = RECT { left: title.left, right: title.right, top: rect.top + self.scaled(27),
                bottom: rect.bottom - margin };
            let width = area.right - area.left;
            let height = area.bottom - area.top;
            if width <= 0 || height <= 0 { return true }
            if let Some(Some(thumb)) = self.extras.board_image_thumbs.get(&card.id) {
                let (d_width, d_height) = fitted_image(thumb.width, thumb.height, width, height);
                let mut bitmap: BITMAPINFO = std::mem::zeroed();
                bitmap.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
                bitmap.bmiHeader.biWidth = thumb.width;
                bitmap.bmiHeader.biHeight = -thumb.height;
                bitmap.bmiHeader.biPlanes = 1;
                bitmap.bmiHeader.biBitCount = 32;
                bitmap.bmiHeader.biCompression = BI_RGB;
                SetStretchBltMode(dc, COLORONCOLOR);
                let lines = StretchDIBits(dc,
                    area.left + (width - d_width) / 2, area.top + (height - d_height) / 2,
                    d_width, d_height, 0, 0, thumb.width, thumb.height, thumb.bgra.as_ptr().cast(),
                    &bitmap, DIB_RGB_COLORS, SRCCOPY);
                if lines > 0 { return true }
            }
            let status = if self.extras.board_image_thumbs.contains_key(&card.id) {
                "圖片目前無法預覽"
            } else {
                "圖片預覽未載入"
            };
            draw_text(dc, self.fonts[3], SOFT, status, &mut area, DT_WORDBREAK);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_geometry_letterboxes_and_preserves_at_least_one_pixel() {
        assert_eq!(fitted_image(100, 50, 40, 40), (40, 20));
        assert_eq!(fitted_image(1, 4096, 64, 64), (1, 64));
        assert_eq!(fitted_image(4096, 1, 64, 64), (64, 1));
    }

    #[test]
    fn preview_cache_only_looks_one_viewport_before_or_after_the_visible_area() {
        assert_eq!(thumbnail_distance(-30, 112, 400), Some(0));
        assert_eq!(thumbnail_distance(455, 600, 400), Some(55));
        assert_eq!(thumbnail_distance(-600, -399, 400), Some(399));
        assert_eq!(thumbnail_distance(801, 1000, 400), None);
        assert_eq!(thumbnail_distance(-1000, -401, 400), None);
        assert!(MAX_CACHED_THUMBS * THUMB_SIDE as usize * THUMB_SIDE as usize * 4 <= 12 * 1024 * 1024);
    }
}
