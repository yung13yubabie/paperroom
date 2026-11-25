//! Explicit highlights refer to UTF-8 ranges in the saved Markdown, never delimiters.
use super::{Notebook, Result};
use pulldown_cmark::{Event, Options, Parser, Tag};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use std::ops::Range;

const MAX_MARKS: usize = 128;
const MAX_SELECTION_BYTES: usize = 8 * 1024;

pub(super) const SCHEMA: &str = "
    CREATE TABLE note_highlights(
        note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
        start_byte INTEGER NOT NULL CHECK(start_byte >= 0),
        end_byte INTEGER NOT NULL CHECK(end_byte > start_byte AND end_byte <= 4194304
                                        AND end_byte - start_byte <= 8192),
        PRIMARY KEY(note_id, start_byte)
    );
    CREATE TRIGGER clear_changed_note_highlights AFTER UPDATE OF body ON notes
    WHEN OLD.body IS NOT NEW.body
    BEGIN
        DELETE FROM note_highlights WHERE note_id=NEW.id;
    END;";

fn source_range(raw: &str, editor: &str, selection: Range<usize>) -> Option<Range<usize>> {
    if selection.start >= selection.end
        || raw.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n") != editor
    {
        return None;
    }

    let mut start = None;
    let mut units = 0;
    let mut chars = raw.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        if units == selection.start { start = Some(offset); }
        if units == selection.end { return Some(start?..offset); }
        if units > selection.end { return None; }
        units += match ch {
            '\r' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\n') {
                    chars.next();
                }
                2
            }
            '\n' => 2,
            _ => ch.len_utf16(),
        };
    }
    (units == selection.end).then_some(start?..raw.len())
}

fn plain_paragraph_range(body: &str, selection: &Range<usize>) -> bool {
    let Some(selected) = body.get(selection.clone()) else { return false };
    if selected.trim().is_empty() || selected.len() > MAX_SELECTION_BYTES
        || selected.chars().any(char::is_control)
        || selected.contains(['\\', '<', '>', '`', '[', ']', '='])
    {
        return false;
    }

    let options = Options::ENABLE_WIKILINKS | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS | Options::ENABLE_GFM | Options::ENABLE_TABLES;
    let mut depth = 0;
    let mut paragraph = false;
    let mut contains_selection = false;
    let mut contains_html = false;
    for (event, raw) in Parser::new_ext(body, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 { paragraph = matches!(tag, Tag::Paragraph); }
                depth += 1;
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    if contains_selection { return !contains_html; }
                    contains_html = false;
                }
            }
            Event::InlineHtml(_) | Event::Html(_) if paragraph => contains_html = true,
            Event::Text(text) if paragraph && depth == 1 && raw.start <= selection.start
                && selection.end <= raw.end && body.get(raw.clone()) == Some(text.as_ref()) =>
            {
                // CommonMark offsets for an escaped character can omit its escape.
                contains_selection = !body[..raw.start].ends_with('\\');
            }
            _ => {}
        }
    }
    false
}

pub(super) fn read_marks(db: &Connection, id: i64, body: &str) -> Result<Vec<Range<usize>>> {
    let mut query = db.prepare_cached(
        "SELECT h.start_byte,h.end_byte FROM note_highlights h JOIN notes n ON n.id=h.note_id
         WHERE h.note_id=?1 AND n.body=?2 AND n.trash_root IS NULL ORDER BY h.start_byte LIMIT ?3",
    )?;
    let mut rows = query.query(params![id, body, MAX_MARKS as i64 + 1])?;
    let mut marks: Vec<Range<usize>> = Vec::new();
    while let Some(row) = rows.next()? {
        let start = usize::try_from(row.get::<_, i64>(0)?)?;
        let end = usize::try_from(row.get::<_, i64>(1)?)?;
        if marks.len() == MAX_MARKS || start >= end || body.get(start..end).is_none()
            || marks.last().is_some_and(|mark| mark.end > start)
        {
            return Err("這篇筆記的高亮位置資料無效。".into());
        }
        marks.push(start..end);
    }
    Ok(marks)
}

fn require_snapshot(db: &Connection, id: i64, version: i64, body: &str) -> Result<()> {
    let current = db.prepare_cached(
        "SELECT 1 FROM notes WHERE id=?1 AND version=?2 AND body=?3 AND trash_root IS NULL",
    )?.exists(params![id, version, body])?;
    if !current { return Err("筆記已變更或移到垃圾桶。請重新開啟後再設定高亮。".into()); }
    Ok(())
}

impl Notebook {
    pub(crate) fn note_highlights(&self, id: i64, body: &str) -> Result<Vec<Range<usize>>> {
        read_marks(&self.connection, id, body)
    }

    pub(crate) fn toggle_note_highlight(
        &self, id: i64, version: i64, body: &str, editor: &str, selection: Range<usize>,
    ) -> Result<bool> {
        let range = source_range(body, editor, selection)
            .filter(|range| plain_paragraph_range(body, range))
            .ok_or("請在已儲存的單行純文字段落選取原文，避開格式、連結或程式碼。")?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        require_snapshot(&tx, id, version, body)?;
        let marks = read_marks(&tx, id, body)?;
        let add = !marks.contains(&range);
        if add {
            if marks.len() == MAX_MARKS {
                return Err("每篇筆記最多可保留 128 處高亮。".into());
            }
            if marks.iter().any(|mark| mark.start < range.end && range.start < mark.end) {
                return Err("選取範圍與既有高亮重疊。請選取原來的完整範圍取消高亮。".into());
            }
            tx.execute(
                "INSERT INTO note_highlights(note_id,start_byte,end_byte) VALUES(?1,?2,?3)",
                params![id, range.start as i64, range.end as i64],
            )?;
        } else {
            tx.execute("DELETE FROM note_highlights WHERE note_id=?1 AND start_byte=?2", params![id, range.start as i64])?;
        }
        tx.commit()?;
        Ok(add)
    }

    pub(crate) fn clear_note_highlights(
        &self, id: i64, version: i64, body: &str, approved: &[Range<usize>],
    ) -> Result<usize> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        require_snapshot(&tx, id, version, body)?;
        if read_marks(&tx, id, body)? != approved {
            return Err("確認後的螢光標記已由另一個視窗修改。請重新確認後再清除。".into());
        }
        let removed = tx.execute("DELETE FROM note_highlights WHERE note_id=?1", [id])?;
        tx.commit()?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Read;

    fn selected(editor: &str, needle: &str) -> Range<usize> {
        let start = editor.find(needle).unwrap();
        let before = editor[..start].encode_utf16().count();
        before..before + needle.encode_utf16().count()
    }

    #[test]
    fn native_ranges_reject_surrogate_midpoints_and_markup() -> Result<()> {
        let raw = "前行\r\n\n高亮🎬後文 ==舊式==\r最後";
        let editor = "前行\r\n\r\n高亮🎬後文 ==舊式==\r\n最後";
        let chosen = selected(editor, "🎬後文");
        let raw_choice = source_range(raw, editor, chosen.clone()).unwrap();
        assert_eq!(&raw[raw_choice.clone()], "🎬後文");
        assert!(plain_paragraph_range(raw, &raw_choice));
        assert!(source_range(raw, editor, chosen.start + 1..chosen.end).is_none());
        assert!(source_range(raw, editor, 3..5).is_none());
        assert!(source_range(raw, "未儲存", chosen).is_none());
        for body in ["==選字==", "[選字](file.md)", "`選字`", "<b>選字</b>", "前 \\*選字", "> 選字", "~~選字~~"] {
            let range = selected(body, if body.starts_with("==") { "==選字==" }
                else if body.starts_with("前") { "*選字" } else { "選字" });
            assert!(!plain_paragraph_range(body, &source_range(body, body, range).unwrap()), "{body}");
        }
        let long = format!("{}🎬繁中", "字".repeat(65_550));
        let range = source_range(&long, &long, selected(&long, "🎬繁")).unwrap();
        assert_eq!(&long[range], "🎬繁");
        Ok(())
    }

    #[test]
    fn clear_requires_the_confirmed_snapshot_of_highlight_positions() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let raw = "第一處 與 第二處";
        let mut first = Notebook::open(&path, false)?;
        let id = first.add(None, "來源", raw)?;
        let version = first.note(id)?.unwrap().version;
        first.toggle_note_highlight(id, version, raw, raw, selected(raw, "第一處"))?;
        let approved = first.note_highlights(id, raw)?;
        let second = Notebook::open(&path, false)?;
        second.toggle_note_highlight(id, version, raw, raw, selected(raw, "第二處"))?;
        assert!(first.clear_note_highlights(id, version, raw, &approved).is_err());
        let current = first.note_highlights(id, raw)?;
        assert_eq!(current.len(), 2);
        assert_eq!(first.clear_note_highlights(id, version, raw, &current)?, 2);
        assert_eq!(first.note(id)?.unwrap().body, raw);
        assert!(second.note_highlights(id, raw)?.is_empty());
        drop(second);
        drop(first);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn migration_from_schema_25_does_not_annotate_legacy_text() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let raw = "舊式 ==螢光== 與反斜線 \\== 原文\r\n\n同名🎬段落";
        let id = {
            let mut db = Notebook::open(&path, false)?;
            let id = db.add(None, "舊稿", raw)?;
            super::super::tests::remove_v26_schema(&db.connection)?;
            db.connection.pragma_update(None, "user_version", 25)?;
            id
        };
        let db = Notebook::open(&path, false)?;
        assert_eq!(db.note(id)?.unwrap().body, raw);
        assert!(db.note_highlights(id, raw)?.is_empty());
        assert!(!crate::markdown::preview_marked(raw, &[]).contains(r"\highlight"));
        assert_eq!(
            db.connection.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
            26,
        );
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn opt_in_roundtrip_preserves_source_and_marks_only_rtf_and_html() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let backup = dir.join("backup.sqlite3");
        let out = dir.join("workspace.zip");
        let raw = "==舊文字==\r\n\n前 {🎬選字} 後\r另段\n\n[原連結](other.md) 和 `code`";
        let displayed = raw.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n");
        let chosen = selected(&displayed, "{🎬選字}");
        let mut db = Notebook::open(&path, false)?;
        let note = db.add(None, "來源", raw)?;
        let old = db.note(note)?.unwrap();
        assert!(db.note_highlights(note, raw)?.is_empty());
        assert!(!crate::markdown::preview(raw).contains(r"\highlight"));
        assert!(db.toggle_note_highlight(note, old.version, raw, &displayed, chosen.clone())?);
        assert_eq!(db.note(note)?.unwrap().body, raw);
        let marks = db.note_highlights(note, raw)?;
        assert_eq!(marks.len(), 1);
        assert_eq!(&raw[marks[0].clone()], "{🎬選字}");
        let rtf = crate::markdown::preview_marked(raw, &marks);
        assert_eq!(rtf.matches(r"\highlight5").count(), 1);
        assert!(rtf.contains(r"{\highlight5 \{\u"));
        assert!(db.toggle_note_highlight(note, old.version, raw, &displayed, chosen.start + 1..chosen.end).is_err());
        db.backup(&backup)?;
        drop(db);

        let db = Notebook::open(&backup, false)?;
        assert_eq!(db.note_highlights(note, raw)?, marks);
        db.export_html_workspace(&out)?;
        let mut archive = zip::ZipArchive::new(File::open(&out)?)?;
        let mut markdown = String::new();
        archive.by_name(&format!("markdown/{note}.md"))?.read_to_string(&mut markdown)?;
        assert_eq!(markdown, raw);
        let mut html = String::new();
        archive.by_name(&format!("pages/{note}.html"))?.read_to_string(&mut html)?;
        assert_eq!(html.matches("<mark>").count(), 1);
        assert!(html.contains("<mark>{🎬選字}</mark>") && html.contains("==舊文字=="));
        assert!(!db.toggle_note_highlight(note, old.version, raw, &displayed, chosen.clone())?);
        assert_eq!(db.note(note)?.unwrap().body, raw);
        assert!(db.toggle_note_highlight(note, old.version, raw, &displayed, chosen.clone())?);
        let renamed = db.save(note, old.version, "改名", raw)?;
        assert_eq!(db.note_highlights(note, raw)?, marks);
        assert!(db.clear_note_highlights(note, old.version, raw, &marks).is_err());
        assert_eq!(db.note_highlights(note, raw)?, marks);
        let changed = format!("異動{raw}");
        db.save(note, renamed, "改名", &changed)?;
        assert!(db.note_highlights(note, raw)?.is_empty());
        assert!(db.note_highlights(note, &changed)?.is_empty());
        drop(archive);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
