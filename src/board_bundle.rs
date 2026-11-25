use super::board_images::{BoardImageInput, ImageSize, create_board_image_note};
use super::{MAX_NOTE_BYTES, Notebook, Result, boards::create_board_on, clean_title, insert, validate_body};
use crate::output::publish_new;
use rusqlite::{Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const CANVAS: &str = "Board.canvas";
const ASCII_ASSET_FORMAT: i64 = 1;
const FORMAT: i64 = 2;
const MAX_CARDS: usize = 128;
const MAX_EDGES: usize = MAX_CARDS * (MAX_CARDS - 1) / 2;
const MAX_CANVAS: usize = 2 * 1024 * 1024;
const MAX_IMAGE: usize = 8 * 1024 * 1024;
const MAX_CONTENT: usize = 128 * 1024 * 1024;
const MAX_ARCHIVE: u64 = (MAX_CONTENT + 4 * 1024 * 1024) as u64;
const NOTE_SIZE: (i32, i32) = (ImageSize::CARD.width, ImageSize::CARD.height);

enum Payload {
    Note(String),
    Image(BoardImageInput),
}

struct ImportCard {
    title: String,
    x: i32,
    y: i32,
    size: Option<ImageSize>,
    payload: Payload,
}

struct Bundle {
    cards: Vec<ImportCard>,
    edges: Vec<(usize, usize)>,
}

impl Notebook {
    pub fn export_board_bundle(&self, board: i64, output: &Path) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM boards WHERE id=?1)",
            [board], |row| row.get(0))?;
        if !exists { return Err("找不到白板。".into()); }
        let native_only: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM board_shapes WHERE board_id=?1)
                 OR EXISTS(SELECT 1 FROM board_tags WHERE board_id=?1)",
            [board], |row| row.get(0),
        )?;
        if native_only {
            return Err("研究 ZIP 目前無法保存白板形狀或白板標籤。請改用工作區備份保留資料。".into());
        }
        let unsupported: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM board_groups WHERE board_id=?1)
                 OR EXISTS(SELECT 1 FROM board_objects WHERE board_id=?1)
                 OR EXISTS(SELECT 1 FROM board_cards c JOIN notes n ON n.id=c.note_id
                           WHERE c.board_id=?1 AND n.trash_root IS NOT NULL)
                 OR EXISTS(SELECT 1 FROM board_cards c JOIN pdf_excerpt_cards e ON e.note_id=c.note_id
                           WHERE c.board_id=?1)",
            [board], |row| row.get(0),
        )?;
        if unsupported {
            return Err("此白板含分區、自訂物件、垃圾桶內卡片或 PDF 摘錄關係，無法完整匯出成研究 ZIP。".into());
        }
        let cards = self.cards_on(board)?;
        if cards.len() > MAX_CARDS { return Err("研究 ZIP 最多可包含 128 張卡片。".into()); }
        let mut nodes = Vec::with_capacity(cards.len());
        let mut files = Vec::with_capacity(cards.len());
        let mut total = 0usize;
        for card in cards {
            if card.id <= 0 { return Err("白板卡片識別碼不符合研究 ZIP 格式。".into()); }
            let note = self.note(card.id)?.ok_or("白板卡片的來源筆記不存在。")?;
            let title = exact_title(&note.row.title)?;
            let attachments: i64 = tx.query_row("SELECT count(*) FROM attachments WHERE note_id=?1",
                [card.id], |row| row.get(0))?;
            let (path, bytes, width, height, name) = if let Some(size) = card.image {
                size.validate()?;
                if !note.body.is_empty() || attachments != 1 {
                    return Err("研究 ZIP 無法同時保存圖片卡的額外正文或附件。".into());
                }
                let (name, bytes) = self.board_image_attachment(card.id)?.ok_or("圖片卡缺少原始圖片。")?;
                let extension = portable_image_extension(&name).ok_or("圖片的檔名不符合可攜格式。")?;
                if image_ext(&bytes) != Some(extension) {
                    return Err("圖片內容和附件副檔名不符。".into());
                }
                (format!("assets/card-{}/{name}", card.id), bytes, size.width, size.height, Some(name))
            } else {
                if attachments != 0 {
                    return Err("研究 ZIP 無法保存一般卡片的附件。".into());
                }
                validate_body(&note.body)?;
                (format!("notes/card-{}.md", card.id), note.body.into_bytes(), NOTE_SIZE.0, NOTE_SIZE.1, None)
            };
            coordinates(card.x, card.y)?;
            add_bytes(&mut total, bytes.len())?;
            nodes.push(json!({
                "id": format!("card-{}", card.id), "type": "file", "file": path,
                "x": card.x, "y": card.y, "width": width, "height": height, "paperroomTitle": title, "paperroomAttachment": name
            }));
            files.push((path, bytes));
        }
        let edges = self.edges_on(board)?.into_iter().map(|edge| json!({
            "id": format!("edge-{}-{}", edge.from, edge.to),
            "fromNode": format!("card-{}", edge.from), "toNode": format!("card-{}", edge.to),
            "fromEnd": "none", "toEnd": "none",
        })).collect::<Vec<_>>();
        let canvas = serde_json::to_vec_pretty(&json!({"paperroomFormat": FORMAT, "nodes": nodes, "edges": edges}))?;
        if canvas.len() > MAX_CANVAS { return Err("Canvas 超出研究 ZIP 的大小上限。".into()); }
        tx.commit()?;
        publish_new(output, |file, _| {
            let mut writer = ZipWriter::new(file);
            write_entry(&mut writer, CANVAS, &canvas)?;
            for (path, bytes) in files {
                write_entry(&mut writer, &path, &bytes)?;
            }
            writer.finish()?;
            Ok(())
        })
    }

    pub fn import_board_bundle(&self, archive: &Path, title: &str) -> Result<i64> {
        let bundle = read_bundle(archive)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let board = create_board_on(&tx, title)?;
        let mut ids = Vec::with_capacity(bundle.cards.len());
        for card in bundle.cards {
            let note = match card.payload {
                Payload::Note(body) => {
                    let note = insert(&tx, None, &card.title, &body)?;
                    tx.execute("INSERT INTO board_cards(board_id,note_id,x,y) VALUES(?1,?2,?3,?4)",
                        params![board, note, card.x, card.y])?;
                    note
                }
                Payload::Image(image) => {
                    let size = card.size.ok_or("圖片卡缺少尺寸。")?;
                    let note = create_board_image_note(&tx, &image)?;
                    tx.execute(
                        "INSERT INTO board_cards(board_id,note_id,x,y,image_width,image_height)
                         VALUES(?1,?2,?3,?4,?5,?6)",
                        params![board, note, card.x, card.y, size.width, size.height],
                    )?;
                    note
                }
            };
            ids.push(note);
        }
        for (from, to) in bundle.edges {
            let from = ids[from];
            let to = ids[to];
            tx.execute("INSERT INTO board_edges(board_id,from_note,to_note) VALUES(?1,?2,?3)",
                params![board, from.min(to), from.max(to)])?;
        }
        tx.commit()?;
        Ok(board)
    }
}

fn exact_title(title: &str) -> Result<&str> {
    if title.chars().any(char::is_control) || clean_title(title) != title {
        return Err("研究 ZIP 的卡片標題無法無損保存。".into());
    }
    Ok(title)
}

fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else {
        None
    }
}

fn coordinates(x: i32, y: i32) -> Result<()> {
    if !(0..=10_000).contains(&x) || !(0..=50_000).contains(&y) {
        return Err("研究 ZIP 卡片的位置超出白板支援範圍。".into());
    }
    Ok(())
}

fn add_bytes(total: &mut usize, size: usize) -> Result<()> {
    *total = total.checked_add(size).filter(|size| *size <= MAX_CONTENT)
        .ok_or("研究 ZIP 的未壓縮內容超過 128 MiB。")?;
    Ok(())
}

fn write_entry(writer: &mut ZipWriter<impl Write + Seek>, path: &str, bytes: &[u8]) -> Result<()> {
    writer.start_file(path, SimpleFileOptions::default().compression_method(CompressionMethod::Stored))?;
    writer.write_all(bytes)?;
    Ok(())
}

fn generated_id(id: &str) -> bool {
    id.strip_prefix("card-").is_some_and(|suffix| {
        !suffix.starts_with('0') && suffix.bytes().all(|byte| byte.is_ascii_digit())
            && suffix.parse::<i64>().is_ok_and(|id| id > 0)
    })
}

fn portable_image_extension(name: &str) -> Option<&'static str> {
    if name.len() > 255 || name.encode_utf16().count() > 255 || name.chars().any(char::is_control)
        || crate::output::windows_output_name(std::ffi::OsStr::new(name)).is_err()
    {
        return None;
    }
    let (_, ext) = name.rsplit_once('.')?;
    if ext.eq_ignore_ascii_case("png") {
        Some("png")
    } else if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
        Some("jpg")
    } else {
        None
    }
}

fn entry_limit(path: &str) -> Option<usize> {
    if path == CANVAS { return Some(MAX_CANVAS); }
    if path.strip_prefix("notes/").and_then(|name| name.strip_suffix(".md")).is_some_and(generated_id) {
        return Some(MAX_NOTE_BYTES);
    }
    let asset = path.strip_prefix("assets/")?;
    if let Some((id, filename)) = asset.split_once('/') {
        return (generated_id(id) && portable_image_extension(filename).is_some()).then_some(MAX_IMAGE);
    }
    let (id, ext) = asset.rsplit_once('.')?;
    (generated_id(id) && matches!(ext, "png" | "jpg")).then_some(MAX_IMAGE)
}

struct OpenBundle {
    archive: ZipArchive<File>,
    entries: BTreeMap<String, usize>,
    _parents: Vec<File>,
}

fn open_bundle(path: &Path) -> Result<OpenBundle> {
    let (mut file, _parents) = super::attachments::open_local_file(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || !(22..=MAX_ARCHIVE).contains(&meta.len()) {
        return Err("研究 ZIP 檔案超過大小上限或格式不符。".into());
    }
    // This format writes a single-disk, non-ZIP64 archive with a bare end
    // record. Compare raw entry count to the indexed count to catch duplicate
    // names even when the ZIP library coalesces their central entries.
    let mut end = [0; 22];
    file.seek(SeekFrom::End(-22))?;
    file.read_exact(&mut end)?;
    let count = usize::from(u16::from_le_bytes([end[10], end[11]]));
    if &end[..4] != b"PK\x05\x06" || end[4..8] != [0; 4] || end[8..10] != end[10..12]
        || end[20..22] != [0; 2] || !(1..=MAX_CARDS + 1).contains(&count)
    {
        return Err("研究 ZIP 的結尾或項目數不符合支援格式。".into());
    }
    let directory_size = u64::from(u32::from_le_bytes(end[12..16].try_into()?));
    let directory_offset = u64::from(u32::from_le_bytes(end[16..20].try_into()?));
    if directory_size + directory_offset + 22 != meta.len() {
        return Err("研究 ZIP 的中央目錄位置不符。".into());
    }
    file.rewind()?;
    let mut archive = ZipArchive::new(file)?;
    if archive.len() != count || archive.offset() != 0
        || archive.central_directory_start() != directory_offset || archive.has_overlapping_entries()?
    {
        return Err("研究 ZIP 含有重複、交疊或不完整的項目。".into());
    }
    let mut entries = BTreeMap::new();
    let mut total = 0usize;
    for index in 0..count {
        let entry = archive.by_index(index)?;
        let name = entry.name()?;
        let size = usize::try_from(entry.size())?;
        let limit = entry_limit(&name).ok_or("研究 ZIP 含非預期的路徑或檔案。")?;
        if entry.name_raw() != name.as_bytes() || !entry.is_file() || entry.is_symlink()
            || entry.unix_mode().is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000))
            || entry.encrypted() || !matches!(entry.compression(), CompressionMethod::Stored | CompressionMethod::Deflated)
            || size > limit || entry.compressed_size() > MAX_ARCHIVE
            || entries.insert(name.to_string(), size).is_some()
        {
            return Err("研究 ZIP 含無法安全讀取的項目。".into());
        }
        if name != CANVAS { add_bytes(&mut total, size)?; }
    }
    Ok(OpenBundle { archive, entries, _parents })
}

fn read_entry(archive: &mut ZipArchive<File>, path: &str, size: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    archive.by_name(path)?.take(size as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() != size {
        return Err("研究 ZIP 項目的實際大小與宣告不符。".into());
    }
    Ok(bytes)
}

fn object_fields(value: &Value, allowed: &[&str]) -> Result<()> {
    if !value.as_object().is_some_and(|item| {
        item.len() == allowed.len() && allowed.iter().all(|key| item.contains_key(*key))
    }) {
        return Err("Canvas 含缺少欄位或不支援的功能。".into());
    }
    Ok(())
}

fn field_string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    Ok(value[field].as_str().ok_or("Canvas 缺少有效文字欄位。")?)
}

fn field_i32(value: &Value, field: &str) -> Result<i32> {
    let number = value[field].as_i64().ok_or("Canvas 缺少有效的整數欄位。")?;
    Ok(i32::try_from(number)?)
}

fn read_bundle(path: &Path) -> Result<Bundle> {
    let OpenBundle { mut archive, mut entries, _parents } = open_bundle(path)?;
    let size = entries.remove(CANVAS).ok_or("研究 ZIP 缺少 Board.canvas。")?;
    let root: Value = serde_json::from_slice(&read_entry(&mut archive, CANVAS, size)?)?;
    object_fields(&root, &["paperroomFormat", "nodes", "edges"])?;
    let version = root["paperroomFormat"].as_i64()
        .filter(|version| matches!(*version, ASCII_ASSET_FORMAT | FORMAT))
        .ok_or("研究 ZIP 格式版本不相容。")?;
    let nodes = root["nodes"].as_array().filter(|rows| rows.len() <= MAX_CARDS)
        .ok_or("Canvas 卡片清單無效或已超過 128 張。")?;
    let edge_rows = root["edges"].as_array().filter(|rows| rows.len() <= MAX_EDGES)
        .ok_or("Canvas 連線清單無效或超出上限。")?;
    let mut cards = Vec::with_capacity(nodes.len());
    let mut ids = BTreeMap::new();
    for node in nodes {
        object_fields(node, &["id", "type", "file", "x", "y", "width", "height", "paperroomTitle", "paperroomAttachment"])?;
        let id = field_string(node, "id")?;
        if !generated_id(id) || node["type"].as_str() != Some("file")
            || ids.insert(id, cards.len()).is_some()
        {
            return Err("Canvas 含非檔案卡片或重複識別碼。".into());
        }
        let path = field_string(node, "file")?;
        let extension = if path == format!("notes/{id}.md") {
            "md"
        } else if version == ASCII_ASSET_FORMAT {
            if path == format!("assets/{id}.png") { "png" }
            else if path == format!("assets/{id}.jpg") { "jpg" }
            else { return Err("Canvas 卡片引用的檔案不在支援路徑。".into()); }
        } else {
            let filename = field_string(node, "paperroomAttachment")?;
            if path != format!("assets/{id}/{filename}") {
                return Err("Canvas 卡片引用的檔案與附件名稱不符。".into());
            }
            portable_image_extension(filename).ok_or("Canvas 無法使用這個附件檔名。")?
        };
        let length = entries.remove(path).ok_or("研究 ZIP 缺少 Canvas 引用的檔案。")?;
        let title = exact_title(field_string(node, "paperroomTitle")?)?.to_owned();
        let x = field_i32(node, "x")?;
        let y = field_i32(node, "y")?;
        coordinates(x, y)?;
        let width = field_i32(node, "width")?;
        let height = field_i32(node, "height")?;
        let bytes = read_entry(&mut archive, path, length)?;
        let (payload, size) = if extension == "md" {
            if (width, height) != NOTE_SIZE || !node["paperroomAttachment"].is_null() { return Err("Canvas 一般筆記卡片尺寸不相容。".into()); }
            let body = String::from_utf8(bytes)?;
            validate_body(&body)?;
            (Payload::Note(body), None)
        } else {
            if image_ext(&bytes) != Some(extension) { return Err("圖片原檔與 Canvas 副檔名不符。".into()); }
            let size = ImageSize { width, height };
            size.validate()?;
            let filename = field_string(node, "paperroomAttachment")?;
            let image = BoardImageInput::new(title.clone(), bytes)?.with_filename(filename)?;
            (Payload::Image(image), Some(size))
        };
        cards.push(ImportCard { title, x, y, size, payload });
    }
    if !entries.is_empty() { return Err("研究 ZIP 含未由 Canvas 使用的檔案。".into()); }
    let mut edges = Vec::with_capacity(edge_rows.len());
    let mut edge_ids = BTreeSet::new();
    let mut pairs = BTreeSet::new();
    for row in edge_rows {
        object_fields(row, &["id", "fromNode", "toNode", "fromEnd", "toEnd"])?;
        let id = field_string(row, "id")?;
        let from = field_string(row, "fromNode")?;
        let to = field_string(row, "toNode")?;
        if id != format!("edge-{}-{}", from.strip_prefix("card-").unwrap_or(""),
            to.strip_prefix("card-").unwrap_or(""))
            || !edge_ids.insert(id) || row["fromEnd"].as_str() != Some("none")
            || row["toEnd"].as_str() != Some("none")
        {
            return Err("Canvas 含重複或不支援的連線。".into());
        }
        let from = *ids.get(from).ok_or("Canvas 連線缺少起點。")?;
        let to = *ids.get(to).ok_or("Canvas 連線缺少終點。")?;
        if from == to || !pairs.insert((from.min(to), from.max(to))) {
            return Err("Canvas 含自我連線或重複連線。".into());
        }
        edges.push((from, to));
    }
    Ok(Bundle { cards, edges })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    use std::fs;

    const BODY: &str = "中文與 emoji 😀\r\n# Source Markdown\r\n\r\n字面 [[參照]]\r\n";
    const PIXELS: [u8; 8] = [9, 25, 80, 120, 220, 180, 40, 255];

    fn image_bytes() -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&PIXELS, 2, 1, image::ExtendedColorType::Rgba8)?;
        Ok(bytes)
    }

    fn write_test_zip(zip: &Path, canvas: &Value, files: &[(&str, &[u8])]) -> Result<()> {
        let mut writer = ZipWriter::new(File::create(zip)?);
        write_entry(&mut writer, CANVAS, &serde_json::to_vec(canvas)?)?;
        for (path, bytes) in files {
            write_entry(&mut writer, path, bytes)?;
        }
        writer.finish()?;
        Ok(())
    }

    #[test]
    fn research_zip_never_silently_drops_native_shapes_or_board_tags() -> Result<()> {
        use super::super::board_shapes::{ShapeKind, ShapeRect};
        let dir = super::super::tests::data_dir();
        let mut db = Notebook::open(&dir.join("board.sqlite3"), false)?;
        let shaped = db.create_board("純形狀白板")?;
        let tagged = db.create_board("純標籤白板")?;
        let ordinary = db.create_board("可匯出白板")?;
        let note = db.add(None,"正常卡片","測試")?;
        for board in [shaped,tagged,ordinary] { db.add_card_on(board,note)?; }
        db.create_board_shape(shaped,ShapeKind::Ellipse,ShapeRect {
            x:520,y:140,width:140,height:80,
        })?;
        db.set_board_tag(tagged,"觀察",true)?;
        for (board,name) in [(shaped,"形狀.zip"),(tagged,"標籤.zip")] {
            let output = dir.join(name);
            let error = db.export_board_bundle(board,&output).unwrap_err();
            assert!(error.to_string().contains("請改用工作區備份"));
            assert!(!output.exists());
        }
        let output = dir.join("正常.zip");
        db.export_board_bundle(ordinary,&output)?;
        assert!(output.is_file());
        Ok(())
    }

    #[test]
    fn workspace_backup_keeps_shapes_tag_edges_and_original_image_bytes() -> Result<()> {
        use super::super::board_shapes::{ShapeKind, ShapeRect};
        use super::super::board_tags::TagSubject;

        let dir = super::super::tests::data_dir();
        let source = dir.join("source.sqlite3");
        let backup = dir.join("backup.sqlite3");
        let refused = dir.join("would-lose-board-data.zip");
        let bytes = image_bytes()?;
        let rectangle = ShapeRect { x: 44, y: 85, width: 180, height: 91 };
        let ellipse = ShapeRect { x: 411, y: 104, width: 132, height: 76 };
        let (board, sibling, note, image, rect_id, ellipse_id) = {
            let mut db = Notebook::open(&source, false)?;
            let board = db.create_board("田野觀察")?;
            let sibling = db.create_board("另一張研究板")?;
            let note = db.add(None, "田野觀察", BODY)?;
            db.add_card_on(board, note)?;
            let image = db.add_board_images(
                board,
                &[BoardImageInput::new("研究附圖.png".into(), bytes.clone())?],
            )?[0];
            db.add_edge_on(board, image, note)?;
            let rect_id = db.create_board_shape(board, ShapeKind::Rectangle, rectangle)?;
            let ellipse_id = db.create_board_shape(board, ShapeKind::Ellipse, ellipse)?;
            db.set_board_tag(board, "#拍攝/夜景", true)?;
            db.set_board_tag(sibling, "#拍攝/夜景", true)?;
            assert!(db.export_board_bundle(board, &refused).unwrap_err().to_string()
                .contains("請改用工作區備份"));
            assert!(!refused.exists());
            db.backup(&backup)?;
            (board, sibling, note, image, rect_id, ellipse_id)
        };
        {
            let copy = Notebook::open(&backup, false)?;
            assert!(!copy.connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
            assert_eq!(copy.note(note)?.unwrap().body.as_bytes(), BODY.as_bytes());
            assert_eq!(copy.board_image_bytes(image)?.unwrap(), bytes);
            assert_eq!(copy.list_attachments(image)?[0].name, "研究附圖.png");
            let edges = copy.edges_on(board)?;
            assert_eq!(edges.len(), 1);
            assert_eq!((edges[0].from, edges[0].to), (note, image));

            let shapes = copy.board_shapes(board)?;
            assert_eq!(shapes.len(), 2);
            assert!(shapes.iter().any(|shape| (shape.id, shape.kind, shape.rect)
                == (rect_id, ShapeKind::Rectangle, rectangle)));
            assert!(shapes.iter().any(|shape| (shape.id, shape.kind, shape.rect)
                == (ellipse_id, ShapeKind::Ellipse, ellipse)));
            assert!(copy.board_shapes(sibling)?.is_empty());
            assert_eq!(copy.board_tags(board)?, ["拍攝/夜景".to_owned()]);

            copy.set_board_tag(board, "#拍攝/夜景", false)?;
            let hits = copy.exact_board_and_note_tag_hits("拍攝/夜景", &[])?;
            assert_eq!(hits.iter().map(|hit| hit.subject).collect::<Vec<_>>(),
                [TagSubject::Board(sibling)]);
        }
        let original = Notebook::open(&source, false)?;
        assert_eq!(original.board_tags(board)?, ["拍攝/夜景".to_owned()]);
        drop(original);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn research_zip_keeps_markdown_image_bytes_layout_and_edges_when_reopened() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("board.sqlite3");
        let archive = dir.join("paperroom.zip");
        let bytes = image_bytes()?;
        assert!(portable_image_extension("片場 參考.PNG").is_some());
        assert!(portable_image_extension(&format!("{}.png", "🎬".repeat(64))).is_none());
        assert!(portable_image_extension("上一層/逃逸.png").is_none());
        let geometry = ImageSize { width: 381, height: 202 };
        let imported = {
            let mut db = Notebook::open(&path, false)?;
            let board = db.create_board("研究")?;
            let note = db.add(None, "研究札記", BODY)?;
            db.add_card_on(board, note)?;
            db.move_card_on(board, note, 7, 301)?;
            let image = db.add_board_images(board, &[BoardImageInput::new("研究附圖.png".into(), bytes.clone())?])?[0];
            db.move_card_on(board, image, 312, 43)?;
            db.resize_board_image(board, image, (312, 43), ImageSize::IMAGE, geometry)?;
            let before_rename = db.note(image)?.ok_or("找不到合成圖片卡")?;
            db.save(image, before_rename.version, "研究重點", &before_rename.body)?;
            db.add_edge_on(board, image, note)?;
            db.export_board_bundle(board, &archive)?;
            assert!(db.export_board_bundle(board, &archive).is_err());

            let OpenBundle { archive: mut opened, entries: paths, _parents } = open_bundle(&archive)?;
            let note_path = format!("notes/card-{note}.md");
            let asset_path = format!("assets/card-{image}/研究附圖.png");
            assert_eq!(paths.len(), 3);
            assert_eq!(read_entry(&mut opened, &note_path, paths[&note_path])?, BODY.as_bytes());
            assert_eq!(read_entry(&mut opened, &asset_path, paths[&asset_path])?, bytes);
            let manifest: Value = serde_json::from_slice(&read_entry(&mut opened, CANVAS, paths[CANVAS])?)?;
            assert_eq!(manifest["nodes"][0]["type"], "file");
            assert_eq!(manifest["nodes"][0]["file"].as_str(), Some(note_path.as_str()));
            assert_eq!(manifest["paperroomFormat"], FORMAT);
            assert_eq!(manifest["nodes"][1]["file"], asset_path);
            assert_eq!(manifest["nodes"][1]["paperroomTitle"], "研究重點");
            let parsed = crate::canvas::parse(String::from_utf8(read_entry(&mut opened, CANVAS, paths[CANVAS])?)?)?;
            assert_eq!(parsed.nodes, 2);
            assert_eq!(manifest["nodes"][1]["paperroomAttachment"], "研究附圖.png");
            assert_eq!(manifest["edges"][0]["fromEnd"], "none");
            assert_eq!(manifest["edges"][0]["toEnd"], "none");
            db.import_board_bundle(&archive, "匯入白板")?
        };
        {
            let reopened = Notebook::open(&path, false)?;
            let cards = reopened.cards_on(imported)?;
            assert_eq!(cards.len(), 2);
            let (text, image) = (&cards[0], &cards[1]);
            assert_eq!((&*text.title, text.x, text.y, text.size()), ("研究札記", 7, 301, ImageSize::CARD));
            assert_eq!((&*image.title, image.x, image.y, image.size()), ("研究重點", 312, 43, geometry));
            assert_eq!(reopened.note(text.id)?.unwrap().body.as_bytes(), BODY.as_bytes());
            assert_eq!(reopened.board_image_bytes(image.id)?.unwrap(), bytes);
            assert_eq!(reopened.list_attachments(image.id)?[0].name, "研究附圖.png");
            let edges = reopened.edges_on(imported)?;
            assert_eq!((edges[0].from, edges[0].to), (text.id, image.id));
        }
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn research_zip_rejects_missing_assets_bad_paths_and_transaction_failures_without_partials() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let mut db = Notebook::open(&dir.join("board.sqlite3"), false)?;
        let good = dir.join("good.zip");
        let broken = dir.join("broken.zip");
        let bytes = image_bytes()?;
        let canvas = json!({"paperroomFormat": 1, "nodes":[
            {"id":"card-1","type":"file","file":"notes/card-1.md","x":1,"y":2,"width":232,"height":114,"paperroomTitle":"正文","paperroomAttachment":null},
            {"id":"card-2","type":"file","file":"assets/card-2.png","x":11,"y":21,"width":232,"height":156,"paperroomTitle":"圖片","paperroomAttachment":"原始圖.png"}
        ], "edges":[{"id":"edge-1-2","fromNode":"card-1","toNode":"card-2","fromEnd":"none","toEnd":"none"}]});
        let baseline = db.rows(super::super::View::All, "")?.len();
        for bad in [
            vec![("notes/card-1.md", BODY.as_bytes())],
            vec![("notes/card-1.md", BODY.as_bytes()), ("assets/Card-2.png", &bytes)],
            vec![("notes/card-1.md", BODY.as_bytes()), ("assets/card-2.png", &bytes),
                 ("../escape", &bytes)],
            vec![("notes/card-1.md", BODY.as_bytes()), ("assets/card-2.png", &bytes),
                 ("assets/card-3.png", &bytes)],
        ] {
            write_test_zip(&broken, &canvas, &bad)?;
            assert!(db.import_board_bundle(&broken, "不可留下").is_err());
            assert_eq!(db.list_boards()?.len(), 1);
            assert_eq!(db.rows(super::super::View::All, "")?.len(), baseline);
        }
        let mut unsupported = canvas.clone();
        unsupported["edges"][0]["toEnd"] = json!("arrow");
        write_test_zip(&broken, &unsupported, &[("notes/card-1.md", BODY.as_bytes()), ("assets/card-2.png", &bytes)])?;
        assert!(db.import_board_bundle(&broken, "不可留下").is_err());

        write_test_zip(&good, &canvas, &[("notes/card-1.md", BODY.as_bytes()), ("assets/card-2.png", &bytes)])?;
        db.connection.execute_batch(
            "CREATE TEMP TRIGGER refuse_research_zip_edge BEFORE INSERT ON board_edges
             BEGIN SELECT RAISE(ABORT,'synthetic link failure'); END;",
        )?;
        assert!(db.import_board_bundle(&good, "無法寫入連線").is_err());
        assert_eq!(db.list_boards()?.len(), 1);
        assert_eq!(db.rows(super::super::View::All, "")?.len(), baseline);
        assert_eq!(db.connection.query_row("SELECT count(*) FROM attachments", [], |row| row.get::<_,i64>(0))?, 0);

        let blocked = dir.join("unsupported.zip");
        let board = db.create_board("含分區")?;
        db.create_board_group(board, "尚未支援")?;
        assert!(db.export_board_bundle(board, &blocked).is_err());
        assert!(!blocked.exists());
        let bare = db.create_board("附帶原稿")?;
        let note = db.add(None, "含附件", "")?;
        let file = dir.join("memo.txt");
        fs::write(&file, "附件內容")?;
        db.import_attachment(note, &file)?;
        db.add_card_on(bare, note)?;
        assert!(db.export_board_bundle(bare, &blocked).is_err());
        assert!(!blocked.exists());
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
