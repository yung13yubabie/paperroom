//! Bounded CSV-to-notes import. No network or schema inference.
use super::{Notebook, Result, insert, MAX_TITLE_CHARS};
use rusqlite::{params, Transaction, TransactionBehavior};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

pub(crate) const MAX_CSV_BYTES: usize = 8 * 1024 * 1024;
const MAX_ROWS: usize = 5000;
const MAX_COLS: usize = 128;
const MAX_FIELD: usize = 4096;

pub(crate) fn parse_csv(bytes: &[u8]) -> Result<Vec<Vec<String>>> {
    parse_csv_with_purpose(bytes, false)
}

/// Read the same bounded CSV format for a preview; do not require a note title
/// or data records because no notes are created.
pub(crate) fn parse_csv_preview(bytes: &[u8]) -> Result<Vec<Vec<String>>> {
    parse_csv_with_purpose(bytes, true)
}

fn parse_csv_with_purpose(bytes: &[u8], preview: bool) -> Result<Vec<Vec<String>>> {
    if bytes.len() > MAX_CSV_BYTES { return Err("CSV 超過 8 MiB。".into()); }
    let text = std::str::from_utf8(bytes)?.trim_start_matches('\u{feff}');
    if text.contains('\0') { return Err("CSV 不能包含 NUL。".into()); }
    let b = text.as_bytes();
    let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), Vec::new());
    let (mut i, mut quoted, mut closed, mut at_start) = (0, false, false, true);
    while i < b.len() {
        let c = b[i];
        if quoted {
            if c == b'"' {
                if b.get(i + 1) == Some(&b'"') { field.push(b'"'); i += 1; }
                else { quoted = false; closed = true; }
            } else { field.push(c); }
        } else if c == b'"' && at_start {
            quoted = true; at_start = false;
        } else if c == b',' || c == b'\n' || c == b'\r' {
            if c == b'\r' {
                if b.get(i + 1) != Some(&b'\n') { return Err("CSV 的列分隔不正確。".into()); }
                i += 1;
            }
            row.push(String::from_utf8(std::mem::take(&mut field))?);
            if row.len() > MAX_COLS { return Err("CSV 欄位超過 128 欄。".into()); }
            if c != b',' {
                rows.push(std::mem::take(&mut row));
                if rows.len() > MAX_ROWS + 1 { return Err("CSV 資料超過 5000 列。".into()); }
            }
            closed = false; at_start = true;
        } else {
            if closed || c == b'"' { return Err("CSV 引號格式不正確。".into()); }
            field.push(c); at_start = false;
        }
        if field.len() > MAX_FIELD { return Err("CSV 欄位超過 4096 位元組。".into()); }
        i += 1;
    }
    if quoted { return Err("CSV 引號尚未結束。".into()); }
    if !row.is_empty() || !field.is_empty() || closed || (!b.is_empty() && b[b.len()-1] == b',') {
        row.push(String::from_utf8(field)?);
        if row.len() > MAX_COLS { return Err("CSV 欄位超過 128 欄。".into()); }
        rows.push(row);
    }
    if rows.is_empty() || rows.len() > MAX_ROWS + 1 || (!preview && rows.len() < 2) {
        return Err(if preview { "CSV 需要欄名，資料最多 5000 列。" }
            else { "CSV 需要欄名及 1 至 5000 列資料。" }.into());
    }
    let width = rows[0].len();
    if rows.iter().any(|r| r.len() != width) { return Err("CSV 每列欄位數必須一致。".into()); }
    let mut seen = HashSet::new();
    for header in &mut rows[0] {
        *header = super::properties::key_checked(header)?.to_owned();
        if !seen.insert(header.clone()) {
            return Err("CSV 欄名不可空白、重複或超過 256 位元組。".into());
        }
    }
    for row in rows[1..].iter().filter(|_| !preview) {
        let title = row[0].trim();
        if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
            return Err("每列第一欄需有標題，且不得超過 180 字。".into());
        }
    }
    Ok(rows)
}

pub(super) fn insert_row(tx: &Transaction<'_>, parent: i64, headers: &[String], row: &[String]) -> Result<i64> {
    let id = insert(tx, Some(parent), &row[0], "")?;
    for (key, value) in headers.iter().zip(row) {
        super::typed_properties::validate_write(tx, key, value)?;
        tx.execute("INSERT INTO note_properties(note_id,key,value) VALUES (?1,?2,?3)", params![id,key,value])?;
    }
    Ok(id)
}

impl Notebook {
    pub fn import_csv_file(&mut self, name: &str, path: &Path) -> Result<(i64, usize)> {
        let f = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        f.take((MAX_CSV_BYTES + 1) as u64).read_to_end(&mut bytes)?;
        self.import_csv(name, &bytes)
    }

    pub fn import_csv(&mut self, name: &str, bytes: &[u8]) -> Result<(i64, usize)> {
        if name.trim().is_empty() || name.trim().chars().count() > MAX_TITLE_CHARS || name.contains('\0') {
            return Err("匯入名稱不可空白或超過 180 字。".into());
        }
        let rows = parse_csv(bytes)?;
        let count = rows.len() - 1;
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let root = insert(&tx, None, name, "")?;
        for row in &rows[1..] {
            insert_row(&tx, root, &rows[0], row)?;
        }
        tx.commit()?;
        Ok((root, count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_quotes_newlines_and_transaction() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let (root, count) = db.import_csv("測試", "\u{feff}名稱,內容\r\n甲,\"第一行\r\n第二行,\"\"好\"\"\"\r\n乙,完成\r\n".as_bytes())?;
        assert_eq!(count, 2);
        let rows = db.property_rows("內容", "第二行")?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.parent, Some(root));
        assert_eq!(rows[0].1, "第一行\r\n第二行,\"好\"");
        let before = db.rows(super::super::View::All, "")?.len();
        assert!(db.import_csv("失敗", b"name,value\na,b\nc\n").is_err());
        assert_eq!(db.rows(super::super::View::All, "")?.len(), before);
        for key in ["欄\t名", "欄\n名", "欄\r名"] {
            let data = format!("名稱,\"{key}\"\r\n甲,值\r\n");
            assert!(db.import_csv("無效欄名", data.as_bytes()).is_err());
            assert_eq!(db.rows(super::super::View::All, "")?.len(), before);
        }
        Ok(())
    }
    #[test]
    fn rejects_bad_syntax_and_limits() {
        for b in [&b"x,x\na,b"[..], &b"x,y\na,\"unterminated"[..], &b"x,y\na,\"ok\"junk"[..], &b"x,y\na,b\rc,d"[..], &b"x\n\n"[..]] {
            assert!(parse_csv(b).is_err());
        }
        assert!(parse_csv(&vec![b'a'; MAX_CSV_BYTES + 1]).is_err());
    }
}
