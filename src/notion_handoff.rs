//! Keep original Notion CSV files and Markdown member paths for each ZIP import.
use super::attachments::{MAX_FILE_BYTES, MAX_STORAGE_BYTES};
use super::{Notebook, Result, clean_title, insert};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt::Write;

const MAX_CSV_FILES: usize = 2_000;
const MAX_TOTAL_CSV_BYTES: usize = 32 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_DEPTH: usize = 64;
const FOLDER: &str = "Notion 原始 CSV";
const SCOPE: &str = "CSV 附件保留匯出檔的原始位元組。打開來源筆記，從「管理附件」可直接將選取的 CSV 建成新集合。預設匯入保留文字；需要數字、核取或日期時，可選「依工作區欄位型別建立集合」並核對對應。CSV 的公式與關聯不會重建。Markdown 連結保留原路徑。可從筆記連結開啟同次匯入、同一 ZIP 部件內精確配對的頁面。";

#[derive(Default)]
pub(crate) struct CsvSources {
    pub(crate) typed_relations: Option<super::notion_typed_relations::Plan>,
    pub(super) page_origins: BTreeMap<Vec<String>, (Option<String>, String)>,
    namespaces: BTreeSet<String>,
    files: BTreeMap<(String, String), CsvSource>,
    bytes: usize,
}

struct CsvSource {
    part: Option<String>,
    path: String,
    bytes: Vec<u8>,
}

impl CsvSource {
    fn location(&self) -> String {
        origin(self.part.as_deref(), &self.path)
    }

    fn filename(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

pub(super) fn windows_zip_path(path: &str) -> bool {
    path.len() <= MAX_PATH_BYTES
        && path.split('/').count() <= MAX_PATH_DEPTH
        && path.split('/').all(|part| {
            !matches!(part, "" | "." | ".." | "__MACOSX")
                && part.encode_utf16().count() <= 255
                && !part.chars().any(|c| c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
                && crate::output::windows_output_name(OsStr::new(part)).is_ok()
        })
}

fn origin(part: Option<&str>, path: &str) -> String {
    part.map_or_else(|| format!("外層 ZIP／{path}"), |part| format!("內嵌 ZIP／{part}!/{path}"))
}

fn abbreviated(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut short = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        short.push('…');
    }
    short
}

pub(crate) fn is_notion_csv(name: &str, source: Option<&str>) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("csv"))
        && source.is_some_and(|origin| origin.starts_with("外層 ZIP／") || origin.starts_with("內嵌 ZIP／"))
}

#[derive(PartialEq)]
pub(crate) struct CsvTypePreview {
    source_note: i64,
    attachment: i64,
    token: String,
    filename: String,
    origin: String,
    bytes: Vec<u8>,
    fields: Vec<(String, Option<super::typed_properties::PropertyType>)>,
    count: usize,
}

impl CsvTypePreview {
    pub(crate) fn summary(&self) -> String {
        let mut text = format!("原始檔：{}\r\n{}\r\n共 {} 列。CSV 欄名移除前後空白後，套用工作區同名欄位的型別：\r\n", self.filename, self.origin, self.count);
        for (name, kind) in &self.fields {
            if let Some(kind) = kind {
                let _ = write!(text, "\r\n{name}：{}", kind.hint());
            }
        }
        text.push_str("\r\n\r\n其餘欄位保留文字。型別欄的空儲存格保持未設定。原始 CSV 仍可另存。");
        text
    }
}

fn read_csv(db: &rusqlite::Connection, source_note: i64, attachment: i64)
    -> Result<(CsvTypePreview, Vec<Vec<String>>)> {
    let source = db.query_row(
        "SELECT a.name,o.relative_path,a.content,i.token FROM attachments a
         JOIN attachment_origins o ON o.attachment_id=a.id
         JOIN notes n ON n.id=a.note_id JOIN note_link_identities i ON i.note_id=n.id
         WHERE a.id=?1 AND a.note_id=?2 AND a.kind='text'
           AND n.trash_root IS NULL AND length(a.content) BETWEEN 1 AND ?3",
        params![attachment, source_note, MAX_FILE_BYTES as i64],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?, row.get::<_, String>(3)?)),
    ).optional()?;
    let (filename, origin, bytes, token) = source.ok_or("找不到使用中的 Notion CSV 原檔。")?;
    if !is_notion_csv(&filename, Some(&origin)) {
        return Err("此附件沒有可辨認的 Notion CSV 來源。".into());
    }
    let rows = super::csv_table::parse_csv(&bytes)?;
    let count = rows.len() - 1;
    if count > super::collections::MAX_ITEMS as usize {
        return Err("每個集合最多 1000 列。原始 CSV 仍可另存。".into());
    }
    let fields = rows[0].iter().map(|name| Ok((name.clone(), super::typed_properties::lookup(db, name)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok((CsvTypePreview { source_note, attachment, token, filename, origin, bytes, fields, count }, rows))
}

impl Notebook {
    pub(crate) fn preview_typed_notion_csv(&self, source_note: i64, attachment: i64) -> Result<CsvTypePreview> {
        let tx = self.connection.unchecked_transaction()?;
        let (preview, rows) = read_csv(&tx, source_note, attachment)?;
        if preview.fields.iter().all(|(_, kind)| kind.is_none()) {
            return Err("沒有相符的工作區型別欄位。請先設定屬性型別，或選擇文字匯入。".into());
        }
        for (index, row) in rows.iter().skip(1).enumerate() {
            for ((name, kind), value) in preview.fields.iter().zip(row) {
                if let Some(kind) = kind && !value.is_empty() && kind.normalized(value).is_none() {
                    return Err(format!("資料第 {} 列的「{name}」需要{}。未建立集合。", index + 1, kind.hint()).into());
                }
            }
        }
        tx.commit()?;
        Ok(preview)
    }

    pub(crate) fn create_typed_notion_csv_collection(&mut self, name: &str, preview: &CsvTypePreview) -> Result<(i64, usize)> {
        self.create_notion_csv_collection(preview.source_note, preview.attachment, name, Some(preview))
    }

    pub(crate) fn create_collection_from_notion_csv(
        &mut self, source_note: i64, attachment: i64, name: &str,
    ) -> Result<(i64, usize)> {
        self.create_notion_csv_collection(source_note, attachment, name, None)
    }

    fn create_notion_csv_collection(&mut self, source_note: i64, attachment: i64, name: &str,
        expected: Option<&CsvTypePreview>) -> Result<(i64, usize)> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (current, rows) = read_csv(&tx, source_note, attachment)?;
        let count = current.count;
        if let Some(preview) = expected {
            if preview != &current {
                return Err("原始 CSV、來源身分或工作區欄位型別已變更。請重新預覽；未建立集合。".into());
            }
        } else if let Some((header, _)) = current.fields.iter().find(|(_, kind)| kind.is_some()) {
            return Err(format!("「{header}」已在工作區宣告為非文字屬性。原始 CSV 已保留。").into());
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE name=?1)", [name.trim()], |row| row.get(0),
        )?;
        if exists {
            return Err("已有同名集合。請使用其他名稱，現有集合與原始 CSV 均已保留。".into());
        }
        let collection = super::collections::insert_collection(&tx, name)?;
        let folder = insert(&tx, Some(source_note), name, "由上層筆記的原始 Notion CSV 建立的可編輯資料列。")?;
        let mut member = tx.prepare_cached(
            "INSERT INTO collection_notes(collection_id,note_id,status) VALUES(?1,?2,'未分類')",
        )?;
        for row in &rows[1..] {
            let note = if expected.is_some() {
                let id = insert(&tx, Some(folder), &row[0], "")?;
                for ((name, kind), value) in current.fields.iter().zip(row) {
                    if kind.is_some() && value.is_empty() { continue; }
                    super::typed_properties::validate_write(&tx, name, value)?;
                    super::properties::write_value(&tx, id, name, value, true)?;
                }
                id
            } else {
                super::csv_table::insert_row(&tx, folder, &rows[0], row)?
            };
            member.execute(params![collection, note])?;
        }
        drop(member);
        tx.commit()?;
        Ok((collection, count))
    }
}

impl CsvSources {
    pub(crate) fn record_page_origin(&mut self, keys: &[String], part: Option<&str>, path: &str) {
        if windows_zip_path(path) {
            self.page_origins.insert(keys.to_vec(), (part.map(str::to_owned), path.into()));
        }
    }

    pub(crate) fn add_namespace(&mut self, part: &str) -> Result<()> {
        if !windows_zip_path(part) || !self.namespaces.insert(part.to_lowercase()) {
            return Err("Notion 內嵌 ZIP 部件名稱不安全或重複。未執行匯入。".into());
        }
        Ok(())
    }

    pub(crate) fn check_member(&self, part: Option<&str>, path: &str, size: u64, symlink: bool) -> Result<()> {
        if symlink || !windows_zip_path(path) {
            return Err("Notion ZIP 內含不安全的 CSV 路徑或符號連結。未執行匯入。".into());
        }
        if self.files.contains_key(&(part.unwrap_or("").to_lowercase(), path.to_lowercase())) {
            return Err(format!("Notion CSV 來源路徑重複：{}。未執行匯入。", abbreviated(&origin(part, path), 180)).into());
        }
        if size == 0 || size > MAX_FILE_BYTES as u64 {
            return Err(format!("Notion CSV 原檔必須非空且不得超過 8 MiB：{}。", abbreviated(&origin(part, path), 180)).into());
        }
        if self.files.len() >= MAX_CSV_FILES || size > MAX_TOTAL_CSV_BYTES.saturating_sub(self.bytes) as u64 {
            return Err("Notion CSV 原檔上限為 2,000 份、合計 32 MiB。未執行匯入。".into());
        }
        Ok(())
    }

    pub(crate) fn capture(&mut self, part: Option<&str>, path: &str, bytes: Vec<u8>) -> Result<()> {
        if std::str::from_utf8(&bytes).is_err() || bytes.contains(&0) {
            return Err(format!("Notion CSV 必須是 UTF-8，且不能含有 NUL：{}。未執行匯入。", abbreviated(&origin(part, path), 180)).into());
        }
        let key = (part.unwrap_or("").to_lowercase(), path.to_lowercase());
        self.bytes += bytes.len();
        self.files.insert(key, CsvSource { part: part.map(str::to_owned), path: path.into(), bytes });
        Ok(())
    }

    pub(crate) fn preview_members(&self) -> impl Iterator<Item = (Option<&str>, &str, &[u8])> {
        self.files.values().map(|s| (s.part.as_deref(), s.path.as_str(), s.bytes.as_slice()))
    }

    pub(crate) fn count(&self) -> usize {
        self.files.len()
    }

    pub(crate) fn report(&self) -> String {
        let mut text = format!("Notion CSV 原檔 {} 份，共 {} 位元組。\r\n{SCOPE}", self.count(), self.bytes);
        if let Some(plan) = &self.typed_relations {
            let _ = write!(text, "\r\n已確認的紙間交接具名關聯：{} 條。", plan.count());
        }
        if self.files.is_empty() {
            return text;
        }
        let _ = write!(text, "\r\n在「{FOLDER}」資料夾中可逐份檢查來源及另存附件。資料夾若同名，會加上序號。\r\n");
        for (i, source) in self.files.values().take(80).enumerate() {
            let _ = write!(text, "\r\n{}　{}", i + 1, abbreviated(&origin(source.part.as_deref(), &source.path), 240));
        }
        if self.files.len() > 80 {
            let _ = write!(text, "\r\n另有 {} 份可在資料夾查閱。", self.files.len() - 80);
        }
        text
    }

    pub(crate) fn attach(self, tx: &Transaction<'_>, root: i64) -> Result<()> {
        if let Some(plan) = &self.typed_relations { plan.attach(tx, root)?; }
        if self.files.is_empty() {
            return Ok(());
        }
        let (used, existing): (i64, i64) = tx.query_row(
            "WITH RECURSIVE imported(id) AS (
                 SELECT ?1 UNION ALL SELECT n.id FROM notes n JOIN imported i ON i.id=n.parent
             ) SELECT (SELECT coalesce(sum(length(content)),0) FROM attachments),
                      (SELECT count(*)-1 FROM imported)",
            [root],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if existing.saturating_add(1 + self.files.len() as i64) > crate::archive::MAX_ENTRIES as i64 {
            return Err("合併 Markdown 與 CSV 後的筆記數超過 15,000。未執行匯入。".into());
        }
        if used > MAX_STORAGE_BYTES.saturating_sub(self.bytes as i64) {
            return Err("工作區附件容量上限為 128 MiB。未執行匯入。".into());
        }

        let mut names = tx.prepare("SELECT title FROM notes WHERE parent=?1")?;
        let existing = names.query_map([root], |row| row.get::<_, String>(0))?
            .map(|title| title.map(|title| title.to_lowercase()))
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        let mut name = FOLDER.to_owned();
        let mut n = 2;
        while existing.contains(&name.to_lowercase()) {
            name = format!("{FOLDER}（{n}）");
            n += 1;
        }
        let folder = insert(tx, Some(root), &name, SCOPE)?;
        let mut attach = tx.prepare("INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,?2,'text',?3)")?;
        let mut source_path = tx.prepare("INSERT INTO attachment_origins(attachment_id,relative_path) VALUES(?1,?2)")?;
        for (i, source) in self.files.into_values().enumerate() {
            let display = origin(source.part.as_deref(), &source.path);
            let title = clean_title(&format!("{}｜{}", i + 1, source.filename()));
            let note = insert(tx, Some(folder), &title, &format!(
                "原始 CSV 路徑\n\n    {display}\n\n附件：{} 位元組。可從附件選單另存原檔。", source.bytes.len(),
            ))?;
            attach.execute(params![note, source.filename(), &source.bytes])?;
            source_path.execute(params![tx.last_insert_rowid(), source.location()])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{archive_with_media, notion_archive_with_media};
    use crate::store::{Notebook, View};
    use std::fs;
    use std::io::{Cursor, Write as _};
    use std::path::Path;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    fn zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options)?;
            writer.write_all(bytes)?;
        }
        Ok(writer.finish()?.into_inner())
    }

    const TABLE_PATH: &str = "專案 3d295461a2b24c53ba33fcca941ed976/書目 09ed986604d54ef2810781f6bc3403ec_all.csv";
    const FIRST: &str = "\u{feff}名稱,關聯,備註\r\n鏡頭,https://example.invalid/notion/09ed986604d54ef2810781f6bc3403ec,\"第一行\r\n第二行有,\"\"引號\"\"\"\r\n";
    const SECOND: &str = "名稱,關聯\r\n不同部件,https://example.invalid/other\r\n";
    const OUTER: &str = "名稱,關聯\r\n外層同名,https://example.invalid/root\r\n";

    #[test]
    fn saved_notion_csv_builds_editable_collection_and_keeps_source_bytes() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let archive = dir.join("test-notion.zip");
        fs::write(&archive, zip(&[
            (TABLE_PATH, FIRST.as_bytes()),
            ("專案.md", b"[child](project/child.md)"),
        ])?)?;
        let original_zip = fs::read(&archive)?;
        let path = dir.join("test-notion.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let (pages, media, sources) = notion_archive_with_media(&archive)?;
        let root = db.import_notion_with_media("匯入", pages.pages, media, sources)?;
        let note = db.rows(View::All, "")?.into_iter().find(|r| r.title.ends_with("_all.csv")).unwrap();
        let attachment = db.list_attachments(note.id)?[0].id;
        let (collection, count) = db.create_collection_from_notion_csv(note.id, attachment, "書目集合")?;
        assert_eq!(count, 1);
        let members = db.collection_rows(collection)?;
        assert_eq!(members.len(), 1);
        let row = &members[0].0;
        let fields = db.properties(row.id)?;
        assert!(fields.iter().any(|(key, value)| key == "備註" && value == "第一行\r\n第二行有,\"引號\""));
        assert!(fields.iter().any(|(key, value)| key == "關聯" && value.starts_with("https://example.invalid/")));
        let folder = db.note(row.parent.unwrap())?.unwrap();
        assert_eq!(folder.row.parent, Some(note.id));
        assert_ne!(folder.row.id, root);
        assert_eq!(db.connection.query_row("SELECT count(*) FROM note_relations WHERE source_id=?1", [row.id], |r| r.get::<_, i64>(0))?, 0);
        assert!(db.create_collection_from_notion_csv(note.id, attachment, "書目集合").is_err());
        assert!(db.create_collection_from_notion_csv(root, attachment, "錯誤來源").is_err());

        db.set_property(row.id, "備註", Some("編輯後"))?;
        drop(db);
        let db = Notebook::open(&path, false)?;
        assert_eq!(db.collection_rows(collection)?.len(), 1);
        assert_eq!(db.property_rows("備註", "編輯後")?.len(), 1);
        let copy = dir.join("copy.csv");
        db.save_attachment_copy(attachment, &copy)?;
        assert_eq!(fs::read(copy)?, FIRST.as_bytes());
        assert_eq!(fs::read(archive)?, original_zip);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn typed_csv_preview_preserves_rows_source_and_definitions_through_restore() -> Result<()> {
        use crate::store::typed_properties::PropertyType::{Checkbox, Date, Number};
        let dir = super::super::tests::data_dir();
        let zip_path = dir.join("Transfer.zip");
        let original = "名稱,時長,完成,拍攝日,代碼,文字空欄\r\n鏡頭,0,false,2028-02-29,0001,\r\n鏡頭,9007199254740993,true,,0002,";
        fs::write(&zip_path, zip(&[("Shots.csv",original.as_bytes())])?)?;
        let path = dir.join("live.sqlite3");
        let mut db = Notebook::open(&path,false)?;
        let (pages, media, sources) = notion_archive_with_media(&zip_path)?;
        db.import_notion_with_media("範例", pages.pages, media, sources)?;
        let source = db.rows(View::All, "")?.into_iter().find(|row| row.title=="1｜Shots.csv").unwrap().id;
        let file = db.list_attachments(source)?[0].id;
        assert!(db.preview_typed_notion_csv(source,file).is_err());
        for (key, kind) in [("時長",Number),("完成",Checkbox),("拍攝日",Date)] {
            db.compare_set_property_type(key,None,Some(kind))?;
        }
        let preview = db.preview_typed_notion_csv(source,file)?;
        assert!(preview.summary().contains("共 2 列"));
        assert!(db.create_collection_from_notion_csv(source,file,"原文字模式").is_err());
        let before = db.rows(View::All,"")?.len();
        db.compare_set_property_type("代碼",None,Some(Number))?;
        assert!(db.create_typed_notion_csv_collection("過期型別",&preview).is_err());
        db.compare_set_property_type("代碼",Some(Number),None)?;
        assert_eq!(db.rows(View::All,"")?.len(),before);
        assert!(db.collections()?.is_empty());
        let (cid, rows) = db.create_typed_notion_csv_collection("可編輯分鏡",&preview)?;
        assert_eq!(rows,2);
        let members = db.collection_rows(cid)?;
        let mut by_code = BTreeMap::new();
        for (row, _) in members {
            assert_eq!(row.title,"鏡頭");
            let fields: BTreeMap<_,_> = db.properties(row.id)?.into_iter().collect();
            assert_eq!(fields["文字空欄"],"");
            by_code.insert(fields["代碼"].clone(),(row.id,fields));
        }
        let first = by_code["0001"].0;
        let second = by_code["0002"].0;
        assert_ne!(first,second);
        assert_eq!(by_code["0001"].1["完成"], "false");
        assert_eq!(by_code["0001"].1["時長"], "0");
        assert_eq!(by_code["0001"].1["拍攝日"], "2028-02-29");
        assert!(!by_code["0002"].1.contains_key("拍攝日"));
        assert_eq!(by_code["0002"].1["時長"],"9007199254740993");
        assert!(db.compare_set_property(first,"拍攝日",Some("2028-02-29"),Some("2027-02-29")).is_err());
        db.compare_set_property(first,"時長",Some("0"),Some("0012.50"))?;
        drop(db);
        let reopened = Notebook::open(&path,false)?;
        reopened.backup(&dir.join("restore.sqlite3"))?;
        drop(reopened);
        let restored = Notebook::open(&dir.join("restore.sqlite3"),false)?;
        assert_eq!(restored.collection_rows(cid)?.len(),2);
        assert_eq!(restored.properties(first)?.into_iter().collect::<BTreeMap<_,_>>()["時長"], "0012.50");
        assert_eq!(restored.properties(second)?.into_iter().collect::<BTreeMap<_,_>>()["時長"],"9007199254740993");
        restored.save_attachment_copy(file,&dir.join("copy.csv"))?;
        assert_eq!(fs::read(dir.join("copy.csv"))?,original.as_bytes());
        drop(restored);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn typed_csv_refuses_invalid_and_stale_same_named_attachments_without_writes() -> Result<()> {
        use crate::store::typed_properties::PropertyType::Date;
        let mut db = Notebook::open(Path::new(":memory:"),false)?;
        let mut csv = CsvSources::default();
        csv.capture(None,"A/S.csv",b"Name,Date\nA,2028-02-29".to_vec())?;
        csv.capture(None,"B/S.csv",b"Name,Date\nA,2028-02-30".to_vec())?;
        db.import_notion_with_media("來源",Vec::new(),Default::default(),csv)?;
        db.compare_set_property_type("Date",None,Some(Date))?;
        let pairs = db.rows(View::All,"")?.into_iter().filter(|r| r.title.ends_with("｜S.csv"))
            .map(|r| Ok((r.id,db.list_attachments(r.id)?[0].id))).collect::<Result<Vec<_>>>()?;
        assert_eq!(pairs.len(),2);
        let (first, file) = pairs[0];
        assert!(db.preview_typed_notion_csv(first,pairs[1].1).is_err());
        assert!(db.preview_typed_notion_csv(pairs[1].0,pairs[1].1).err().unwrap().to_string().contains("資料第 1 列"));
        let plan = db.preview_typed_notion_csv(first,file)?;
        db.connection.execute("UPDATE attachments SET content=?2 WHERE id=?1",params![file,&b"Name,Date\nA,2020-01-01"[..]])?;
        let before = db.rows(View::All,"")?.len();
        assert!(db.create_typed_notion_csv_collection("已換檔",&plan).is_err());
        assert!(db.collections()?.is_empty());
        assert_eq!(db.rows(View::All,"")?.len(),before);
        Ok(())
    }

    #[test]
    fn saved_notion_csv_refuses_invalid_collisions_and_rolls_back() -> Result<()> {
        use crate::store::typed_properties::PropertyType;
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let mut csv = CsvSources::default();
        csv.capture(None, "a.csv", b"Name,Id\na,0001\nb,0002".to_vec())?;
        csv.capture(None, "b.csv", b"Name,Id\na,0001\nb".to_vec())?;
        db.import_notion_with_media("來源", Vec::new(), Default::default(), csv)?;
        let inputs = db.rows(View::All, "")?.into_iter().filter(|row| row.title.ends_with(".csv"))
            .map(|row| Ok((row.id, db.list_attachments(row.id)?[0].id)))
            .collect::<Result<Vec<_>>>()?;
        let before = db.rows(View::All, "")?.len();
        db.compare_set_property_type("Id", None, Some(PropertyType::Number))?;
        assert!(db.create_collection_from_notion_csv(inputs[0].0, inputs[0].1, "拒絕").unwrap_err().to_string().contains("非文字"));
        db.compare_set_property_type("Id", Some(PropertyType::Number), None)?;
        assert!(db.create_collection_from_notion_csv(inputs[1].0, inputs[1].1, "拒絕").unwrap_err().to_string().contains("欄位數"));
        db.connection.execute_batch("CREATE TEMP TRIGGER reject_csv_member BEFORE INSERT ON collection_notes
            WHEN (SELECT title FROM notes WHERE id=NEW.note_id)='b'
            BEGIN SELECT RAISE(ABORT,'test membership failure'); END;")?;
        assert!(db.create_collection_from_notion_csv(inputs[0].0, inputs[0].1, "拒絕").is_err());
        assert!(db.collections()?.is_empty());
        assert_eq!(db.rows(View::All, "")?.len(), before);
        assert_eq!(db.list_attachments(inputs[0].0)?[0].id, inputs[0].1);
        Ok(())
    }

    #[test]
    fn notion_csv_handoff_keeps_namespaces_relative_media_and_exact_saved_bytes() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let input = dir.join("ExportBlock.zip");
        let path = dir.join("browse.sqlite");
        let backup = dir.join("backup.sqlite");
        let project = "專案 3d295461a2b24c53ba33fcca941ed976";
        let parent = format!("{project}.md");
        let child = format!("{project}/鏡頭.md");
        let media_path = format!("{project}/assets/demo.pdf");
        let pdf = b"%PDF-1.7\nsynthetic";
        let primary = zip(&[
            (&parent, "# 專案\n\n原始父頁".as_bytes()),
            (&child, "# 鏡頭\n\n[參考](assets/demo.pdf)".as_bytes()),
            (&media_path, pdf),
            ("Notion 原始 CSV.md", "# Notion 原始 CSV\n\n真實 Markdown 筆記保留。".as_bytes()),
            (TABLE_PATH, FIRST.as_bytes()),
        ])?;
        let second = zip(&[(TABLE_PATH, SECOND.as_bytes())])?;
        let lookalike = format!("Part-1.zip!/{TABLE_PATH}");
        let archive = zip(&[
            ("Part-1.zip", &primary), ("Part-2.zip", &second), (&lookalike, OUTER.as_bytes()),
        ])?;
        fs::write(&input, &archive)?;
        let mut db = Notebook::open(&path, false)?;
        let (input_plan, media, csv) = notion_archive_with_media(&input)?;
        assert_eq!((input_plan.pages.len(), input_plan.skipped, media.count(), csv.count()), (3, 0, 1, 3));
        assert!(csv.report().contains(&format!("外層 ZIP／{lookalike}")));
        assert!(csv.report().contains(&format!("內嵌 ZIP／Part-1.zip!/{TABLE_PATH}")));
        assert!(csv.report().contains(&format!("內嵌 ZIP／Part-2.zip!/{TABLE_PATH}")));
        assert!(db.rows(View::All, "")?.is_empty());
        let root = db.import_notion_with_media("原始包", input_plan.pages, media, csv)?;
        let rows = db.rows(View::All, "")?;
        let id_of = |name: &str| rows.iter().find(|row| row.title == name).unwrap().id;
        let folder = id_of("Notion 原始 CSV（2）");
        assert_eq!(db.note(folder)?.unwrap().row.parent, Some(root));
        assert_eq!(db.note(id_of("專案"))?.unwrap().row.parent, Some(root));
        let markdown = db.note(id_of("鏡頭"))?.unwrap();
        assert_eq!(markdown.row.parent, Some(id_of("專案")));
        assert_eq!(db.note(id_of(FOLDER))?.unwrap().body, "真實 Markdown 筆記保留。");
        let pdf_id = db.list_attachments(markdown.row.id)?[0].id;
        assert_eq!(db.attachment_origin(pdf_id)?.as_deref(), Some(media_path.as_str()));
        assert_eq!(db.read_pdf_attachment(pdf_id)?.1, pdf);
        let sources = rows.iter().filter(|row| row.parent == Some(folder)).map(|row| row.id)
            .collect::<Vec<_>>();
        assert_eq!(sources.len(), 3);
        assert_eq!(fs::read(&input)?, archive);
        db.backup(&backup)?;
        drop(db);
        fs::rename(&input, dir.join("原 ZIP 已移走"))?;

        for sqlite in [&path, &backup] {
            let db = Notebook::open(sqlite, false)?;
            let origins = [(None, lookalike.as_str(), OUTER),
                (Some("Part-1.zip"), TABLE_PATH, FIRST), (Some("Part-2.zip"), TABLE_PATH, SECOND)];
            for (index, (part, source_path, original)) in origins.into_iter().enumerate() {
                let ordinal = index + 1;
                let source = sources.iter().copied().find(|&id|
                    db.meta(id).unwrap().unwrap().title.starts_with(&format!("{ordinal}｜"))).unwrap();
                let expected = origin(part, source_path);
                assert!(db.note(source)?.unwrap().body.contains(&expected));
                let attachments = db.list_attachments(source)?;
                assert_eq!(attachments.len(), 1);
                assert_eq!(attachments[0].name, TABLE_PATH.rsplit('/').next().unwrap());
                let attachment = attachments[0].id;
                assert_eq!(db.attachment_origin(attachment)?.as_deref(), Some(expected.as_str()));
                let copy = dir.join(format!("另存 {ordinal}.csv"));
                db.save_attachment_copy(attachment, &copy)?;
                assert_eq!(fs::read(&copy)?, original.as_bytes());
                fs::remove_file(copy)?;
            }
        }
        let mut db = Notebook::open(&backup, false)?;
        let (materialized, count) = db.import_csv("手動轉入資料", FIRST.as_bytes())?;
        assert_eq!(count, 1);
        let url = db.property_rows("關聯", "example.invalid")?;
        assert_eq!(url[0].0.parent, Some(materialized));
        assert_eq!(url[0].1, "https://example.invalid/notion/09ed986604d54ef2810781f6bc3403ec");
        let multiline = db.property_rows("備註", "第二行")?;
        assert_eq!(multiline[0].1, "第一行\r\n第二行有,\"引號\"");
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn notion_csv_only_reads_and_legacy_zip_does_not_treat_csv_as_media() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("source.zip");
        fs::write(&path, zip(&[("書目.csv", FIRST.as_bytes())])?)?;
        assert!(archive_with_media(&path).is_err());
        let (plan, media, sources) = notion_archive_with_media(&path)?;
        assert!(plan.pages.is_empty());
        assert_eq!(sources.count(), 1);
        assert!(sources.report().contains("外層 ZIP／書目.csv"));
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        db.import_notion_with_media("表格包", plan.pages, media, sources)?;
        assert_eq!(db.rows(View::All, "")?.len(), 3);

        fs::write(&path, zip(&[("筆記.md", "[原檔](書目.csv)".as_bytes()), ("書目.csv", FIRST.as_bytes())])?)?;
        let (legacy, media) = archive_with_media(&path)?;
        assert_eq!((legacy.pages.len(), legacy.skipped, media.count(), media.unresolved), (1, 1, 0, 0));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn notion_csv_rejects_invalid_inputs_and_rolls_back_if_a_source_cannot_be_saved() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("source.zip");
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        db.add(None, "原有筆記", "保留")?;
        for bad in ["../escape.csv", "CON.csv", "報表.csv ", "資料\\漏出.csv"] {
            fs::write(&path, zip(&[("筆記.md", b"new"), (bad, FIRST.as_bytes())])?)?;
            assert!(notion_archive_with_media(&path).is_err(), "{bad}");
        }
        fs::write(&path, zip(&[("same.csv", FIRST.as_bytes()), ("SAME.CSV", SECOND.as_bytes())])?)?;
        assert!(notion_archive_with_media(&path).is_err());
        let part = zip(&[("表.csv", FIRST.as_bytes())])?;
        fs::write(&path, zip(&[("Part.zip", &part), ("PART.ZIP", &part)])?)?;
        assert!(notion_archive_with_media(&path).is_err());
        for invalid in [b"".as_slice(), &[0xff, 0xfe], b"embedded\0null".as_slice()] {
            fs::write(&path, zip(&[("表.csv", invalid)])?)?;
            assert!(notion_archive_with_media(&path).is_err());
        }
        let oversized = vec![b'x'; MAX_FILE_BYTES + 1];
        fs::write(&path, zip(&[("大檔.csv", &oversized)])?)?;
        assert!(notion_archive_with_media(&path).is_err());
        drop(oversized);
        let mut corrupt = zip(&[("損毀.csv", b"unique-csv-checksum-marker")])?;
        let payload = corrupt.windows(b"unique-csv-checksum-marker".len())
            .position(|bytes| bytes == b"unique-csv-checksum-marker").unwrap();
        corrupt[payload] = b'x';
        fs::write(&path, corrupt)?;
        assert!(notion_archive_with_media(&path).is_err());

        fs::write(&path, zip(&[
            ("筆記.md", b"[attachment](doc.txt)"),
            ("doc.txt", b"test"),
            ("資料.csv", FIRST.as_bytes()),
        ])?)?;
        let (plan, media, csv) = notion_archive_with_media(&path)?;
        assert_eq!(media.count(), 1);
        db.connection.execute_batch(
            "CREATE TEMP TRIGGER reject_source BEFORE INSERT ON attachments WHEN NEW.name LIKE '%.csv'
             BEGIN SELECT RAISE(ABORT, 'test transaction rollback'); END;",
        )?;
        assert!(db.import_notion_with_media("不應出現", plan.pages, media, csv).is_err());
        assert_eq!(db.rows(View::All, "")?.len(), 1);
        let attachments: i64 = db.connection.query_row("SELECT count(*) FROM attachments", [], |row| row.get(0))?;
        assert_eq!(attachments, 0);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
