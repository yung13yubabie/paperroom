//! Treat Notion ZIP paths as identifiers inside one import batch and archive part.
use super::Result;
use crate::store::migration_media::source_relative;
use rusqlite::{Connection, OptionalExtension, params};

pub(super) const SCHEMA: &str = "
CREATE TABLE notion_page_paths (
    note_id INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    import_root INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    archive_part TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    UNIQUE(import_root, archive_part, relative_path)
);";

pub(super) struct Source {
    root: i64,
    part: String,
    parts: Vec<String>,
}

impl Source {
    pub(super) fn load(db: &Connection, note: i64) -> Result<Option<Self>> {
        Ok(db.query_row(
            "SELECT p.import_root, p.archive_part, p.relative_path
             FROM notion_page_paths p JOIN notes root ON root.id=p.import_root
             WHERE p.note_id=?1 AND root.trash_root IS NULL",
            [note],
            |row| {
                let path: String = row.get(2)?;
                Ok(Self {
                    root: row.get(0)?,
                    part: row.get(1)?,
                    parts: path.split('/').map(str::to_owned).collect(),
                })
            },
        ).optional()?)
    }

    pub(super) fn resolve(&self, db: &Connection, raw_path: &str) -> Result<Option<(i64, String)>> {
        if raw_path.is_empty() || raw_path.len() > 4096 || raw_path.contains('?') {
            return Ok(None);
        }
        let Some(path) = source_relative(&self.parts, raw_path) else {
            return Ok(None);
        };
        // The persisted index contains only actual Markdown members. Equality
        // uses their full, case-sensitive archive paths with the extension.
        let id = db.prepare_cached(
            "SELECT p.note_id FROM notion_page_paths p JOIN notes n ON n.id=p.note_id
             WHERE p.import_root=?1 AND p.archive_part=?2 AND p.relative_path=?3
               AND n.trash_root IS NULL",
        )?.query_row(params![self.root, self.part, path], |row| row.get(0)).optional()?;
        Ok(id.map(|id| (id, path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::notion_archive_with_media;
    use crate::markdown::{AnchorSyntax, HeadingLookup, resolve_heading};
    use crate::store::{Notebook, View};
    use std::fs;
    use std::io::{Cursor, Write};
    use std::path::Path;
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn archive(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (path, body) in entries {
            zip.start_file(*path, SimpleFileOptions::default())?;
            zip.write_all(body)?;
        }
        Ok(zip.finish()?.into_inner())
    }

    fn import(db: &mut Notebook, file: &Path) -> Result<i64> {
        let (pages, media, sources) = notion_archive_with_media(file)?;
        db.import_notion_with_media("工作區匯入", pages.pages, media, sources)
    }

    const ENTRY: &str = "甲 11111111111111111111111111111111/入口.md";
    const ALPHA: &str = "甲 11111111111111111111111111111111/鏡頭 A.md";
    const BRAVO: &str = "乙 22222222222222222222222222222222/鏡頭 A.md";
    const PERCENT: &str = "甲 11111111111111111111111111111111/進度%20.md";
    const SOURCE: &str = concat!(
        "[同層](./%E9%8F%A1%E9%A0%AD%20A.md#取景%20甲)\n\n",
        "[父層](../乙%2022222222222222222222222222222222/鏡頭%20A.md#同名)\n\n",
        "[百分號](進度%2520.md) [未知片段](鏡頭%20A.md#notion-block-id) ",
        "[疑似 slug](鏡頭%20A.md#north-take) [區塊標記](鏡頭%20A.md#%5E取景)\n\n",
        "[外部](https://example.invalid/鏡頭%20A.md) [附件](本地.txt) ",
        "[同名但不存在](../空資料夾/鏡頭%20A.md) [大小寫不符](鏡頭%20a.md) ",
        "[另一卷的頁面](../其他部件/孤立.md)\n"
    );

    fn note_at(db: &Notebook, root: i64, path: &str) -> Result<i64> {
        Ok(db.connection.query_row(
            "SELECT note_id FROM notion_page_paths WHERE import_root=?1 AND relative_path=?2",
            params![root, path], |row| row.get(0),
        )?)
    }

    fn assert_links(db: &Notebook, root: i64) -> Result<()> {
        let from = db.note(note_at(db, root, ENTRY)?)?.unwrap();
        let alpha = note_at(db, root, ALPHA)?;
        let bravo = note_at(db, root, BRAVO)?;
        assert_ne!(alpha, bravo);
        let links = db.vault_note_links(&from)?;
        assert_eq!(from.body, SOURCE);
        assert_eq!(links.iter().map(|r| (r.label.as_str(), r.note_id)).collect::<Vec<_>>(), vec![
            ("同層", alpha), ("父層", bravo), ("百分號", note_at(db, root, PERCENT)?),
            ("未知片段", alpha), ("疑似 slug", alpha), ("區塊標記", alpha),
        ]);
        assert!(links.iter().all(|r| r.anchor_syntax == AnchorSyntax::NotionMarkdown));
        assert_eq!(links[0].notion_path.as_deref(), Some(ALPHA));
        assert_eq!(links[1].notion_path.as_deref(), Some(BRAVO));
        for (link, expected) in links.iter().zip(["found", "ambiguous", "", "missing", "missing", "unsupported"]) {
            if let Some(anchor) = &link.anchor {
                let body = db.note(link.note_id)?.unwrap().body;
                match resolve_heading(&body, anchor, link.anchor_syntax) {
                    HeadingLookup::Found { section, .. } => {
                        assert_eq!(expected, "found");
                        assert!(body[section].contains("只在甲中"));
                    }
                    HeadingLookup::Ambiguous => assert_eq!(expected, "ambiguous"),
                    HeadingLookup::Missing => assert_eq!(expected, "missing"),
                    HeadingLookup::Unsupported => assert_eq!(expected, "unsupported"),
                }
            }
        }
        assert_eq!(db.list_attachments(from.row.id)?.len(), 1);
        assert!(db.imported_vaults()?.is_empty());
        Ok(())
    }

    #[test]
    fn explicit_notion_links_keep_source_scope_and_bytes_after_reopen_and_restore() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("workspace.sqlite3");
        let backup = dir.join("backup.sqlite3");
        let restored = dir.join("restored.sqlite3");
        let zip_path = dir.join("notion.zip");
        let pages = archive(&[
            (ENTRY, format!("# 入口\n\n{SOURCE}").as_bytes()),
            (ALPHA, "# 鏡頭\n\n## 取景 甲\n只在甲中\n## North Take\nEnglish heading\n".as_bytes()),
            (BRAVO, "# 鏡頭\n\n## 同名\n乙中甲\n## 同名\n乙中乙\n".as_bytes()),
            (PERCENT, "# 進度\n\n可辨認的百分號".as_bytes()),
            ("甲 11111111111111111111111111111111/本地.txt", "只用本機素材".as_bytes()),
        ])?;
        let other = archive(&[("其他部件/孤立.md", "# 其他部件\n\n獨立的來源".as_bytes())])?;
        let original = archive(&[("Part-1.zip", &pages), ("Part-2.zip", &other)])?;
        fs::write(&zip_path, &original)?;

        let mut db = Notebook::open(&path, false)?;
        let first = import(&mut db, &zip_path)?;
        let second = import(&mut db, &zip_path)?;
        assert_ne!(note_at(&db, first, ALPHA)?, note_at(&db, second, ALPHA)?);
        assert_links(&db, first)?;
        assert_links(&db, second)?;

        let target = note_at(&db, first, ALPHA)?;
        db.trash(target)?;
        let source = db.note(note_at(&db, first, ENTRY)?)?.unwrap();
        assert!(db.vault_note_links(&source)?.iter().all(|link| link.note_id != target));
        db.restore(target)?;
        assert_links(&db, first)?;
        drop(db);

        let db = Notebook::open(&path, false)?;
        assert_links(&db, first)?;
        db.backup(&backup)?;
        drop(db);
        fs::copy(&backup, &restored)?;
        let restored_db = Notebook::open(&restored, false)?;
        assert_links(&restored_db, first)?;
        assert_links(&restored_db, second)?;
        assert_eq!(fs::read(&zip_path)?, original);
        drop(restored_db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn v30_upgrade_leaves_old_notes_unmapped_and_duplicate_archive_fails_before_import() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("legacy.sqlite3");
        let mut old = Notebook::open(&path, false)?;
        let source = old.add(None, "舊筆記", "[同名](鏡頭.md)")?;
        old.add(None, "鏡頭", "同名不代表來源")?;
        super::super::tests::remove_v30_schema(&old.connection)?;
        old.connection.pragma_update(None, "user_version", 30)?;
        drop(old);
        let mut db = Notebook::open(&path, false)?;
        let note = db.note(source)?.unwrap();
        assert!(db.vault_note_links(&note)?.is_empty());
        assert_eq!(note.body, "[同名](鏡頭.md)");

        let dup = dir.join("collision.zip");
        // Both extensions would produce exactly the same imported note path.
        fs::write(&dup, archive(&[("同名.md", b"# one"), ("同名.markdown", b"# two")])?)?;
        let before = db.rows(View::All, "")?.len();
        assert!(import(&mut db, &dup).is_err());
        assert_eq!(db.rows(View::All, "")?.len(), before);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
