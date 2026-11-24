//! A note-backed image keeps the same attachment and card IDs when placed on other boards.
use super::{MAX_TITLE_CHARS, Notebook, Result, attachments, boards, insert};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::{io::Cursor, path::Path};

pub(super) const SCHEMA: &str = "
CREATE TABLE board_image_cards (
    note_id INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    attachment_id INTEGER NOT NULL UNIQUE REFERENCES attachments(id) ON DELETE CASCADE,
    pixel_width INTEGER NOT NULL CHECK(pixel_width BETWEEN 1 AND 4096),
    pixel_height INTEGER NOT NULL CHECK(pixel_height BETWEEN 1 AND 4096)
);
ALTER TABLE board_cards ADD COLUMN image_width INTEGER CHECK(image_width BETWEEN 64 AND 768);
ALTER TABLE board_cards ADD COLUMN image_height INTEGER CHECK(image_height BETWEEN 64 AND 768);
CREATE VIEW board_card_geometry AS
SELECT c.board_id,c.note_id,c.x,c.y,
       coalesce(c.image_width,232) AS width,
       coalesce(c.image_height,CASE WHEN i.note_id IS NULL THEN 114 ELSE 156 END) AS height,
       i.note_id IS NOT NULL AS is_image
FROM board_cards c LEFT JOIN board_image_cards i ON i.note_id=c.note_id;
CREATE TRIGGER clear_removed_board_image_size AFTER DELETE ON board_image_cards BEGIN
    UPDATE board_cards SET image_width=NULL,image_height=NULL WHERE note_id=OLD.note_id;
END;
";

const MAX_IMAGE_SIDE: u32 = 4096;
const MAX_PIXELS: u64 = 12 * 1024 * 1024;
const MAX_IMAGES_PER_PASTE: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ImageSize {
    pub width: i32,
    pub height: i32,
}

impl ImageSize {
    pub const CARD: Self = Self { width: 232, height: 114 };
    pub const IMAGE: Self = Self { width: 232, height: 156 };

    pub fn validate(self) -> Result<()> {
        if (64..=768).contains(&self.width) && (64..=768).contains(&self.height) {
            Ok(())
        } else {
            Err("圖片卡寬與高都須介於 64 到 768。".into())
        }
    }
}

fn image_format(bytes: &[u8]) -> Result<ImageFormat> {
    if bytes.is_empty() || bytes.len() > attachments::MAX_FILE_BYTES {
        return Err("圖片檔案須有內容，且至多 8 MiB。".into());
    }
    if attachments::has_expected_signature("png", bytes) {
        Ok(ImageFormat::Png)
    } else if attachments::has_expected_signature("jpeg", bytes) {
        Ok(ImageFormat::Jpeg)
    } else {
        Err("請選擇有效的 PNG 或 JPEG 圖片。".into())
    }
}

pub(crate) fn decode_image(bytes: &[u8]) -> Result<RgbaImage> {
    let format = image_format(bytes)?;
    decode_with_format(bytes, format)
}

fn decode_with_format(bytes: &[u8], format: ImageFormat) -> Result<RgbaImage> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("圖片總像素超過可載入上限。".into());
    }
    Ok(image::DynamicImage::from_decoder(decoder)?.into_rgba8())
}

pub(crate) struct BoardImageInput {
    title: String,
    filename: String,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    kind: &'static str,
}

impl BoardImageInput {
    pub(crate) fn new(title: String, bytes: Vec<u8>) -> Result<Self> {
        let title = title.trim();
        if title.is_empty() || title.chars().any(char::is_control) || title.chars().count() > MAX_TITLE_CHARS {
            return Err("圖片卡名稱需為 1 至 180 個單行字元。".into());
        }
        let format = image_format(&bytes)?;
        let (width, height) = decode_with_format(&bytes, format)?.dimensions();
        let kind = if format == ImageFormat::Png { "png" } else { "jpeg" };
        let filename = if valid_attachment_name(title, kind) {
            title
        } else if kind == "png" {
            "圖片.png"
        } else {
            "圖片.jpg"
        };
        Ok(Self { title: title.to_owned(), filename: filename.to_owned(), bytes, width, height, kind })
    }

    pub(crate) fn with_filename(mut self, filename: &str) -> Result<Self> {
        if !valid_attachment_name(filename, self.kind) {
            return Err("圖片附件的檔名或副檔名無法保存。".into());
        }
        self.filename = filename.to_owned();
        Ok(self)
    }

    #[cfg(windows)]
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let title = path.file_name().and_then(|part| part.to_str()).ok_or("找不到有效圖片檔名。")?;
        let kind = attachments::file_kind(path)?;
        if !matches!(kind, "png" | "jpeg") {
            return Err("白板圖片貼上支援 PNG 或 JPEG 檔。".into());
        }
        let bytes = attachments::read_local_attachment_bytes(path)?;
        let input = Self::new(super::clean_title(title), bytes)?;
        if kind != input.kind {
            return Err("圖片格式和副檔名不同，未匯入圖片。".into());
        }
        input.with_filename(title)
    }
}

fn valid_attachment_name(name: &str, kind: &str) -> bool {
    name.chars().count() <= 255 && !name.chars().any(char::is_control)
        && crate::output::windows_output_name(std::ffi::OsStr::new(name)).is_ok()
        && attachments::file_kind(Path::new(name)).is_ok_and(|extension| extension == kind)
}

/// Call inside the owning board transaction so incomplete images never appear in the workspace.
pub(super) fn create_board_image_note(
    db: &Connection, entry: &BoardImageInput,
) -> Result<i64> {
    let used: i64 = db.query_row("SELECT coalesce(sum(length(content)),0) FROM attachments", [], |row| row.get(0))?;
    if used > attachments::MAX_STORAGE_BYTES - entry.bytes.len() as i64 {
        return Err("工作區附件已達 128 MiB 上限。".into());
    }
    let note = insert(db, None, &entry.title, "")?;
    db.execute(
        "INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,?2,?3,?4)",
        params![note, entry.filename, entry.kind, entry.bytes],
    )?;
    let attachment = db.last_insert_rowid();
    db.execute(
        "INSERT INTO board_image_cards(note_id,attachment_id,pixel_width,pixel_height) VALUES(?1,?2,?3,?4)",
        params![note, attachment, entry.width, entry.height],
    )?;
    Ok(note)
}

fn install_board_image(db: &Connection, board: i64, entry: &BoardImageInput) -> Result<i64> {
    let note = create_board_image_note(db, entry)?;
    boards::place_card_on(db, board, note)?;
    Ok(note)
}

impl Notebook {
    pub(crate) fn add_board_images(&self, board: i64, entries: &[BoardImageInput]) -> Result<Vec<i64>> {
        if entries.is_empty() || entries.len() > MAX_IMAGES_PER_PASTE {
            return Err("一次可匯入 1 至 16 張圖片。".into());
        }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let notes = entries.iter().map(|entry| install_board_image(&tx, board, entry)).collect::<Result<_>>()?;
        tx.commit()?;
        Ok(notes)
    }

    pub(crate) fn board_image_bytes(&self, note: i64) -> Result<Option<Vec<u8>>> {
        Ok(self.board_image_attachment(note)?.map(|(_, bytes)| bytes))
    }

    pub(super) fn board_image_attachment(&self, note: i64) -> Result<Option<(String, Vec<u8>)>> {
        Ok(self.connection.query_row(
            "SELECT a.name,a.content FROM board_image_cards i
             JOIN attachments a ON a.id=i.attachment_id AND a.note_id=i.note_id
             JOIN notes n ON n.id=i.note_id WHERE i.note_id=?1 AND n.trash_root IS NULL",
            [note], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?)
    }

    pub(crate) fn resize_board_image(
        &self, board: i64, note: i64, expected_at: (i32, i32), expected: ImageSize, next: ImageSize,
    ) -> Result<()> {
        next.validate()?;
        if self.connection.execute(
            "UPDATE board_cards SET image_width=?5,image_height=?6
             WHERE board_id=?1 AND note_id=?2 AND x=?7 AND y=?8
             AND coalesce(image_width,232)=?3 AND coalesce(image_height,156)=?4
             AND EXISTS(SELECT 1 FROM board_image_cards i JOIN notes n ON n.id=i.note_id
                 WHERE i.note_id=?2 AND n.trash_root IS NULL)",
            params![board, note, expected.width, expected.height, next.width, next.height, expected_at.0, expected_at.1],
        )? != 1 {
            return Err("圖片卡已移動或調整大小，請重新整理白板。".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::board_objects::{BoardItem, BoardPosition};

    #[test]
    fn image_card_uses_original_bytes_and_moves_with_its_edges() -> Result<()> {
        use image::ImageEncoder;
        let dir = super::super::tests::data_dir();
        let mut bytes = Vec::new();
        let pixels = [10, 25, 80, 120, 220, 180, 40, 255];
        image::codecs::png::PngEncoder::new(&mut bytes).write_image(&pixels, 2, 1, image::ExtendedColorType::Rgba8)?;
        assert_eq!(decode_image(&bytes)?.into_raw(), pixels);
        let image = BoardImageInput::new("研究圖片.png".into(), bytes.clone())?;
        let db_path = dir.join("board.sqlite3");
        let (first, second, picture, note) = {
            let mut db = Notebook::open(&db_path, false)?;
            let first = db.create_board("來源")?;
            let second = db.create_board("分支")?;
            let note = db.add(None, "研究問題", "")?;
            db.add_card_on(first, note)?;
            assert!(db.add_board_images(-1, &[BoardImageInput::new("錯誤".into(), bytes.clone())?]).is_err());
            assert!(!db.rows(super::super::View::All, "")?.iter().any(|note| note.title == "錯誤"));
            let picture = db.add_board_images(first, &[image])?[0];
            let next = ImageSize { width: 320, height: 198 };
            let placement = db.cards_on(first)?.into_iter().find(|card| card.id == picture).unwrap();
            let at = (placement.x, placement.y);
            db.resize_board_image(first, picture, at, ImageSize::IMAGE, next)?;
            assert!(db.resize_board_image(first, picture, at, ImageSize::IMAGE, next).is_err());
            assert!(db.resize_board_image(first, picture, (at.0+1, at.1), next, ImageSize::IMAGE).is_err());
            db.add_edge_on(first, picture, note)?;
            db.move_card_on(first, picture, 35, 208)?;
            (first, second, picture, note)
        };
        {
            let db = Notebook::open(&db_path, false)?;
            let rows = db.cards_on(first)?;
            let geometry = rows.iter().find(|card| card.id == picture).unwrap();
            assert_eq!((geometry.x, geometry.y, geometry.size()), (35, 208, ImageSize { width: 320, height: 198 }));
            assert_eq!(db.board_image_bytes(picture)?.unwrap(), bytes);
            let expected = rows.iter().map(|card| BoardPosition {
                item: BoardItem::Card(card.id), x: card.x, y: card.y, version: None,
            }).collect::<Vec<_>>();
            db.move_cards_between_boards(first, second, &expected, 0, 0)?;
            assert_eq!(db.edges_on(second)?[0].from, note);
            assert_eq!(db.edges_on(second)?[0].to, picture);
            assert_eq!(db.cards_on(second)?.iter().find(|card| card.id == picture).unwrap().size(), geometry.size());
            assert!(db.cards_on(first)?.is_empty());
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
