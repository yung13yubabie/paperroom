//! Offline Zotero CSL JSON bibliography import.
use super::attachments::{MAX_FILE_BYTES, MAX_STORAGE_BYTES, read_local_attachment_bytes};
use super::{Notebook, Result, clean_title, insert};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Map, Value};
use std::fmt::Write;
use std::path::Path;

const MAX_ITEMS: usize = 5_000;
pub(crate) const ORIGINAL_ATTACHMENT: &str = "Zotero 原始 CSL JSON.txt";
pub(super) const SCHEMA: &str = "
    CREATE TABLE zotero_imports(
        root_note INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
        original_attachment INTEGER NOT NULL UNIQUE REFERENCES attachments(id) ON DELETE CASCADE
    );";

const SCOPE: &str = "保留這次提供的 CSL JSON 書目與原始欄位。這次匯入不會還原 Zotero 的 PDF、附件、標註、收藏庫結構或內部連結。URL 以文字保留。";

struct Record {
    title: String,
    body: String,
}

pub(crate) struct ZoteroImport {
    name: String,
    raw: Vec<u8>,
    records: Vec<Record>,
}

fn item_type(kind: &str) -> bool {
    matches!(kind,
        "article" | "article-journal" | "article-magazine" | "article-newspaper"
        | "bill" | "book" | "broadcast" | "chapter" | "classic" | "collection" | "dataset"
        | "document" | "entry" | "entry-dictionary" | "entry-encyclopedia" | "event"
        | "figure" | "graphic" | "hearing" | "interview" | "legal_case" | "legislation"
        | "manuscript" | "map" | "motion_picture" | "musical_score" | "pamphlet"
        | "paper-conference" | "patent" | "performance" | "personal_communication"
        | "periodical" | "post" | "post-weblog" | "regulation" | "report" | "review"
        | "review-book" | "software" | "song" | "speech" | "standard" | "thesis"
        | "treaty" | "webpage"
    )
}

fn display_line(value: &str, limit: usize) -> String {
    let mut chars = value.trim().chars();
    let mut line: String = chars.by_ref().take(limit).map(|c| {
        if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            ' '
        } else {
            c
        }
    }).collect();
    if chars.next().is_some() {
        line.push('…');
    }
    line
}

fn scalar(value: &Value, limit: usize) -> Option<String> {
    match value {
        Value::String(text) => (!text.trim().is_empty()).then(|| display_line(text, limit)),
        Value::Number(number) => Some(display_line(&number.to_string(), limit)),
        _ => None,
    }
}

fn names(value: &Value) -> Option<String> {
    let people = value.as_array()?;
    let mut output = Vec::new();
    for person in people.iter().take(50) {
        if let Some(literal) = person.get("literal").and_then(|value| scalar(value, 180)) {
            output.push(literal);
        } else {
            let parts: Vec<_> = ["non-dropping-particle", "family", "given", "dropping-particle", "suffix"]
                .iter().filter_map(|key| person.get(key).and_then(|v| scalar(v, 180))).collect();
            if !parts.is_empty() {
                output.push(parts.join(" "));
            }
        }
    }
    if people.len() > 50 {
        output.push("…".into());
    }
    (!output.is_empty()).then(|| output.join("、"))
}

fn numeric_date(parts: &Value) -> Option<String> {
    let parts = parts.as_array()?;
    if !(1..=3).contains(&parts.len()) {
        return None;
    }
    let year = parts[0].as_i64().or_else(|| parts[0].as_str()?.parse::<i64>().ok())?;
    let mut result = year.to_string();
    for part in &parts[1..] {
        let number = part.as_u64().or_else(|| part.as_str()?.parse::<u64>().ok())?;
        let _ = write!(result, "-{number:02}");
    }
    Some(result)
}

fn date(value: &Value) -> Option<String> {
    let mut date = ["literal", "raw"].iter().find_map(|key| value.get(key).and_then(|v| scalar(v, 240)))
        .or_else(|| {
            let dates = value.get("date-parts")?.as_array()?;
            if !(1..=2).contains(&dates.len()) { return None; }
            let ranges = dates.iter().map(numeric_date).collect::<Option<Vec<_>>>()?;
            Some(ranges.join(" 至 "))
        })?;
    if value.get("circa").is_some_and(|v| v.as_bool() == Some(true) || v.as_i64() == Some(1)) {
        date.insert_str(0, "約 ");
    }
    if let Some(season) = value.get("season").and_then(|v| scalar(v, 32)) {
        let _ = write!(date, "（season {season}）");
    }
    Some(date)
}

fn write_field(body: &mut String, label: &str, value: Option<String>) {
    let Some(value) = value else { return; };
    // A delimiter longer than any authored run makes the value literal in
    // reading mode while preserving its characters for raw notebook search.
    let length = value.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1;
    let marker = "`".repeat(length);
    let _ = write!(body, "**{label}**　{marker} {value} {marker}\n\n");
}

fn record(item: &Map<String, Value>, ordinal: usize) -> Result<Record> {
    let bad = || format!("第 {ordinal} 筆缺失或含有無法辨識的 CSL type。");
    let kind = item.get("type").and_then(Value::as_str).filter(|kind| item_type(kind)).ok_or_else(bad)?;
    if item.get("id").is_some_and(|v| !v.is_string() && !v.is_number()) {
        return Err(format!("第 {ordinal} 筆的 CSL id 必須是字串或數字。").into());
    }
    if item.get("title").is_some_and(|v| !v.is_string() && !v.is_null()) {
        return Err(format!("第 {ordinal} 筆的 CSL title 必須是文字。").into());
    }
    let full_title = item.get("title").and_then(|v| scalar(v, 1_000));
    let title = full_title.as_ref().map_or_else(|| format!("未命名書目 {ordinal}"), |v| clean_title(v));
    let mut body = format!("**Zotero CSL JSON**　原始書目第 {ordinal} 筆\n\n");
    write_field(&mut body, "題名", full_title);
    write_field(&mut body, "類型", Some(kind.into()));
    for (key, label) in [("author", "作者"), ("editor", "編者"), ("translator", "譯者")] {
        write_field(&mut body, label, item.get(key).and_then(names));
    }
    for (key, label) in [("issued", "出版日期"), ("accessed", "查閱日期"), ("original-date", "原版日期")] {
        write_field(&mut body, label, item.get(key).and_then(date));
    }
    for (key, label) in [
        ("container-title", "刊物或書名"), ("publisher", "出版者"), ("publisher-place", "出版地"),
        ("volume", "卷"), ("issue", "期"), ("page", "頁碼"), ("edition", "版次"), ("language", "語言"),
        ("DOI", "DOI"), ("ISBN", "ISBN"), ("ISSN", "ISSN"), ("URL", "URL"),
        ("citation-key", "引文鍵"), ("id", "原始識別碼"), ("abstract", "摘要"),
    ] {
        write_field(&mut body, label, item.get(key).and_then(|v| scalar(v, 2_000)));
    }
    body.push_str("完整原始欄位保存在這批匯入根筆記的原始 CSL JSON 附件。");
    Ok(Record { title, body })
}

impl ZoteroImport {
    pub(crate) fn read(path: &Path) -> Result<Self> {
        if !path.extension().and_then(|v| v.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("json")) {
            return Err("請選擇 Zotero 匯出的 CSL JSON 檔案（.json）。".into());
        }
        let name = path.file_stem().and_then(|v| v.to_str()).unwrap_or("Zotero");
        Self::parse(name, read_local_attachment_bytes(path)?)
    }

    fn parse(name: &str, bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
            return Err("CSL JSON 須為非空檔案，大小不可超過 8 MiB。".into());
        }
        std::str::from_utf8(&bytes).map_err(|_| "CSL JSON 必須是 UTF-8。")?;
        let input = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
        let data: Value = serde_json::from_slice(input).map_err(|e| {
            format!("讀不到有效的 CSL JSON。第 {} 行、第 {} 欄：{e}", e.line(), e.column())
        })?;
        let records = match &data {
            Value::Object(item) => vec![record(item, 1)?],
            Value::Array(items) if !items.is_empty() && items.len() <= MAX_ITEMS => {
                items.iter().enumerate().map(|(i, item)| {
                    let row = i + 1;
                    let item = item.as_object().ok_or_else(|| format!("第 {row} 筆 CSL 書目不是 JSON 物件。"))?;
                    record(item, row)
                }).collect::<Result<Vec<_>>>()?
            }
            _ => return Err("需要一個 CSL 書目物件，或含 1 至 5,000 筆書目的 JSON 陣列。".into()),
        };
        let name = clean_title(&format!("Zotero 書目｜{}", display_line(name, 150)));
        Ok(Self { name, raw: bytes, records })
    }

    pub(crate) fn count(&self) -> usize {
        self.records.len()
    }

    pub(crate) fn preview(&self) -> String {
        let mut text = format!("{}\r\n共 {} 筆書目，{} 位元組。\r\n{SCOPE}\r\n\r\n",
            self.name, self.count(), self.raw.len());
        for (index, record) in self.records.iter().take(12).enumerate() {
            let _ = write!(text, "{}. {}\r\n", index + 1, record.title);
        }
        if self.count() > 12 {
            text.push_str("其餘書目將同批匯入。\r\n");
        }
        text.push_str("\r\n同名書目各自建立一篇筆記。批次根筆記保留原始檔快照，可逐位元匯出。");
        text
    }
}

impl Notebook {
    pub(crate) fn import_zotero_csl(&mut self, plan: ZoteroImport) -> Result<i64> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let space_used: i64 = tx.query_row(
            "SELECT coalesce(sum(length(content)),0) FROM attachments", [], |r| r.get(0),
        )?;
        if space_used > MAX_STORAGE_BYTES.saturating_sub(plan.raw.len() as i64) {
            return Err("工作區附件空間不足。未建立 Zotero 書目批次。".into());
        }
        let body = format!("**Zotero CSL JSON**　{} 筆書目\n\n{SCOPE}\n\n各筆書目在下層筆記。從「檔案」選單可匯出這批書目的原始 CSL JSON 快照。", plan.count());
        let root = insert(&tx, None, &plan.name, &body)?;
        for note in plan.records {
            insert(&tx, Some(root), &note.title, &note.body)?;
        }
        tx.execute(
            "INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,?2,'text',?3)",
            params![root, ORIGINAL_ATTACHMENT, plan.raw],
        )?;
        let attachment = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO zotero_imports(root_note,original_attachment) VALUES(?1,?2)",
            params![root, attachment],
        )?;
        tx.commit()?;
        Ok(root)
    }

    pub(crate) fn zotero_original_attachment(&self, root: i64) -> Result<i64> {
        self.connection.query_row(
            "SELECT a.id FROM zotero_imports z JOIN notes n ON n.id=z.root_note
             JOIN attachments a ON a.id=z.original_attachment AND a.note_id=z.root_note
             WHERE z.root_note=?1 AND n.trash_root IS NULL AND a.kind='text'",
            [root], |row| row.get(0),
        ).optional()?.ok_or_else(|| "找不到這個 Zotero 匯入批次的原始 CSL JSON。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    const INPUT: &str = concat!("\u{feff}", r#"[
    {"id":"第一個","type":"article-journal","title":"同名研究",
     "author":[{"family":"林","given":"小文","suffix":"博士"}],
     "issued":{"date-parts":[[2025,4,3]]},"accessed":{"date-parts":[[2026,11,24]]},
     "DOI":"10.1234/test","URL":"https://example.invalid/paper",
     "abstract":"原文字串 ``來源`` ![[紙間可能的嵌入]] <script>保留</script>",
     "extra-nested":{"未知欄":[1.234567891234567890123456789,null,{"位置":"中文"}]}},
    {"id":9,"type":"book","title":"同名研究","author":[{"literal":"編輯團隊"}],
     "issued":{"date-parts":[["2021","05"],[2023]],"circa":1},
     "accessed":{"raw":"2024 年秋季"}}
]"#, "\r\n");

    #[test]
    fn zotero_metadata_retains_duplicate_titles_fields_and_exact_source_on_reopen() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let source = dir.join("合成研究.csl.json");
        let path = dir.join("工作區.sqlite");
        let backup = dir.join("複本.sqlite");
        let export = dir.join("原始匯出.csl.json");
        let portable = dir.join("匯入根附件.zip");
        fs::write(&source, INPUT)?;
        let mut db = Notebook::open(&path, false)?;
        let plan = ZoteroImport::read(&source)?;
        assert_eq!(plan.count(), 2);
        assert!(plan.preview().contains("1. 同名研究") && plan.preview().contains("2. 同名研究"));
        assert!(db.rows(crate::store::View::All, "")?.is_empty());
        let root = db.import_zotero_csl(plan)?;
        let rows = db.rows(crate::store::View::All, "")?;
        let children: Vec<_> = rows.iter().filter(|row| row.parent == Some(root)).collect();
        assert_eq!(children.len(), 2);
        assert_ne!(children[0].id, children[1].id);
        assert!(children.iter().all(|row| row.title == "同名研究"));
        let bodies: Vec<_> = children.iter().map(|row| db.note(row.id).map(|note| note.unwrap().body))
            .collect::<Result<_>>()?;
        let first = bodies.iter().find(|body| body.contains("林 小文 博士")).unwrap();
        let preview = crate::markdown::preview(first);
        for field in ["2025-04-03", "2026-11-24", "10.1234/test", "https://example.invalid/paper"] {
            assert!(preview.contains(field), "{field}");
        }
        assert!(crate::store::transclusion::embed_references(first).next().is_none());
        let hits = db.rows(crate::store::View::All, "10.1234/test")?;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "同名研究");
        let second = bodies.iter().find(|body| body.contains("編輯團隊")).unwrap();
        assert!(second.contains("約 2021-05 至 2023") && second.contains("2024 年秋季"));
        assert!(db.zotero_original_attachment(children[0].id).is_err());
        assert_eq!(db.list_attachments(root)?[0].name, ORIGINAL_ATTACHMENT);
        assert_eq!(fs::read(&source)?, INPUT.as_bytes());
        db.backup(&backup)?;
        assert_eq!(db.export_note_bundle(root, &portable)?, 1);
        drop(db);
        fs::rename(&source, dir.join("原來源已移出"))?;

        for sqlite in [path, backup] {
            let db = Notebook::open(&sqlite, false)?;
            let attachment = db.zotero_original_attachment(root)?;
            db.save_attachment_copy(attachment, &export)?;
            assert_eq!(fs::read(&export)?, INPUT.as_bytes());
            assert!(db.save_attachment_copy(attachment, &export).is_err());
            fs::remove_file(&export)?;
        }
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let (new_root, count) = db.import_note_bundle(&portable)?;
        assert_eq!(count, 1);
        assert!(db.zotero_original_attachment(new_root).is_err());
        let restored = db.list_attachments(new_root)?;
        assert_eq!(restored[0].name, ORIGINAL_ATTACHMENT);
        db.save_attachment_copy(restored[0].id, &export)?;
        assert_eq!(fs::read(&export)?, INPUT.as_bytes());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn zotero_export_requires_original_attachment_identity_even_after_same_name_replacement() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let mimic = dir.join(ORIGINAL_ATTACHMENT);
        fs::write(&mimic, INPUT)?;
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let unrelated = db.add(None, "非匯入批次", "")?;
        db.import_attachment(unrelated, &mimic)?;
        assert!(db.zotero_original_attachment(unrelated).is_err());

        let root = db.import_zotero_csl(ZoteroImport::parse("測試", INPUT.as_bytes().to_vec())?)?;
        let original = db.zotero_original_attachment(root)?;
        let substitute = db.import_attachment(root, &mimic)?;
        assert_ne!(substitute, original);
        assert_eq!(db.zotero_original_attachment(root)?, original);
        db.remove_attachment(original)?;
        assert!(db.zotero_original_attachment(root).is_err());
        assert_eq!(db.list_attachments(root)?[0].id, substitute);
        let readded = db.import_attachment(root, &mimic)?;
        assert_ne!(readded, original);
        assert!(db.zotero_original_attachment(root).is_err());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn zotero_preflight_and_attachment_failure_leave_workspace_untouched() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let existing = db.add(None, "既有紙間筆記", "保留")?;
        let before = db.note(existing)?.unwrap();
        let dir = super::super::tests::data_dir();
        let bad_path = dir.join("錯誤.json");
        let invalid_second_item = r#"[{"id":"valid","type":"book"},{"type":123}]"#;
        for invalid in ["{", "[]", r#"{"items":[]}"#, invalid_second_item] {
            fs::write(&bad_path, invalid)?;
            assert!(ZoteroImport::read(&bad_path).is_err(), "{invalid}");
        }
        let oversized = fs::File::create(&bad_path)?;
        oversized.set_len(MAX_FILE_BYTES as u64 + 1)?;
        drop(oversized);
        assert!(ZoteroImport::read(&bad_path).is_err());
        fs::write(&bad_path, vec![0xff, 0xfe, 0x00])?;
        assert!(ZoteroImport::read(&bad_path).is_err());
        assert!(ZoteroImport::parse("空", Vec::new()).is_err());
        let excessive = format!("[{}]", vec![r#"{"type":"book"}"#; MAX_ITEMS + 1].join(","));
        assert!(ZoteroImport::parse("過多", excessive.into_bytes()).is_err());

        let single = r#"{"id":"ok","type":"thesis","title":"單筆"}"#;
        fs::write(&bad_path, single)?;
        let plan = ZoteroImport::read(&bad_path)?;
        assert_eq!(plan.count(), 1);
        db.connection.execute_batch(
            "CREATE TEMP TRIGGER reject_test_attachment BEFORE INSERT ON attachments
             BEGIN SELECT RAISE(ABORT, 'test transaction rollback'); END;",
        )?;
        assert!(db.import_zotero_csl(plan).is_err());
        assert_eq!(db.rows(crate::store::View::All, "")?.len(), 1);
        assert_eq!(db.note(existing)?.unwrap().body, before.body);
        assert!(db.list_attachments(existing)?.is_empty());
        assert_eq!(fs::read_to_string(&bad_path)?, single);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
