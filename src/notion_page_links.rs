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
    fn chosen_csv_row_source_survives_reopen_backup_and_keeps_exact_attachment() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let zip = dir.join("notion.zip");
        let original = archive(&[
            ("data.csv", "Name,Status\n鏡頭,ready".as_bytes()),
            ("甲/鏡頭.md", "# 鏡頭\n\n甲正文 [圖](圖.txt)".as_bytes()),
            ("乙/鏡頭.md", "# 鏡頭\n\n乙正文 [圖](圖.txt)".as_bytes()),
            ("甲/圖.txt", b"decoy"), ("乙/圖.txt", b"selected image"),
        ])?;
        fs::write(&zip,&original)?;
        let path = dir.join("workspace.sqlite3");
        let mut db = Notebook::open(&path,false)?;
        let first = import(&mut db,&zip)?;
        let csv = db.rows(View::All,"")?.into_iter().find(|r| r.title.ends_with("data.csv")).unwrap();
        let attachment = db.list_attachments(csv.id)?[0].id;
        let (collection,_) = db.create_collection_from_notion_csv(csv.id,attachment,"集合")?;
        let row = db.collection_rows(collection)?[0].0.id;
        let fields = db.properties(row)?;
        let second = import(&mut db,&zip)?;
        let batches = db.notion_source_batches()?;
        let batch = batches.iter().find(|b| b.id == first).unwrap();
        let pages = db.notion_source_pages(batch,row)?;
        let page = pages.iter().find(|p| p.path=="乙/鏡頭.md").unwrap();
        let token = db.notion_row_token(row)?.unwrap();
        assert!(db.bind_notion_source(row,&token,batches.iter().find(|b| b.id==second).unwrap(),page,None).is_err());
        db.trash(page.id)?;
        assert!(db.bind_notion_source(row,&token,batch,page,None).is_err());
        db.restore(page.id)?;
        assert!(db.bind_notion_source(row,"stale",batch,page,None).is_err());
        db.bind_notion_source(row,&token,batch,page,None)?;
        assert!(db.bind_notion_source(row,&token,batch,&pages[0],None).is_err());
        drop(db);
        let db = Notebook::open(&path,false)?;
        let backup = dir.join("backup.sqlite3");
        db.backup(&backup)?;
        drop(db);
        let isolated = dir.join("isolated.sqlite3");
        fs::copy(backup,&isolated)?;
        let db = Notebook::open(&isolated,false)?;
        let origin = db.named_relation_origin(row)?;
        let direction = super::super::named_relation_navigation::Direction::Outgoing;
        let (hits,_) = db.named_relation_page(&origin,direction,0)?;
        assert_eq!(hits.len(),1);
        let note = db.note_from_named_relation(&origin,direction,&hits[0])?;
        assert_eq!(note.row.id,page.id);
        assert!(note.body.contains("乙正文"));
        let copy = dir.join("materialized.txt");
        db.save_attachment_copy(db.list_attachments(note.row.id)?[0].id,&copy)?;
        assert_eq!(fs::read(copy)?,b"selected image");
        assert_eq!(db.properties(row)?,fields);
        assert_eq!(fs::read(zip)?,original);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn replaces_confirmed_source_only_and_rolls_back_failed_insert() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let zip = dir.join("notion.zip");
        fs::write(&zip,archive(&[
            ("old/shot.md", b"# Shot\n\nold [asset](ref.txt)"),
            ("new/shot.md", b"# Shot\n\nnew [asset](ref.txt)"),
            ("old/ref.txt", b"old bytes"), ("new/ref.txt", b"new bytes"),
        ])?)?;
        let path = dir.join("workspace.sqlite3");
        let mut db = Notebook::open(&path,false)?;
        import(&mut db,&zip)?;
        let row = db.add(None,"Shot","standalone row")?;
        let collection = db.create_collection("shots")?;
        db.add_to_collection(collection,row,"ready")?;
        let token = db.notion_row_token(row)?.unwrap();
        let batch = db.notion_source_batches()?.remove(0);
        let pages = db.notion_source_pages(&batch,row)?;
        let (old,new) = (&pages[1],&pages[0]);
        assert_eq!((&*old.path,&*new.path),("old/shot.md","new/shot.md"));
        db.bind_notion_source(row,&token,&batch,old,None)?;
        let previous = db.bound_notion_source(row)?.unwrap();
        let other = db.add(None,"independent","unrelated")?;
        let insert = "INSERT INTO note_relations(source_id,name,target_id) VALUES(?1,?2,?3)";
        db.connection.execute(insert,params![row,"Other",other])?;
        db.connection.execute(insert,params![row,SOURCE_RELATION,other])?;
        assert!(db.bind_notion_source(row,&token,&batch,new,Some(&previous)).is_err());
        db.connection.execute("DELETE FROM note_relations WHERE source_id=?1 AND name=?2 AND target_id=?3",
            params![row,SOURCE_RELATION,other])?;
        db.trash(new.id)?;
        assert!(db.bind_notion_source(row,&token,&batch,new,Some(&previous)).is_err());
        db.restore(new.id)?;
        // SQLite rejects the new edge after the old edge has been removed inside the same transaction.
        db.connection.execute_batch("CREATE TEMP TRIGGER fail_edge BEFORE INSERT ON note_relations
            BEGIN SELECT RAISE(ABORT,'test write failure'); END;")?;
        assert!(db.bind_notion_source(row,&token,&batch,new,Some(&previous)).is_err());
        assert_eq!(db.bound_notion_source(row)?.as_ref(),Some(&previous));
        db.connection.execute_batch("DROP TRIGGER fail_edge;")?;
        db.bind_notion_source(row,&token,&batch,new,Some(&previous))?;
        assert!(db.bind_notion_source(row,&token,&batch,old,Some(&previous)).is_err());
        drop(db);
        let db = Notebook::open(&path,false)?;
        assert!(db.bound_notion_source(row)?.unwrap().matches(new));
        let backup = dir.join("backup.sqlite3");
        db.backup(&backup)?;
        drop(db);
        let isolated = dir.join("isolated.sqlite3");
        fs::copy(backup,&isolated)?;
        let db = Notebook::open(&isolated,false)?;
        assert!(db.bound_notion_source(row)?.unwrap().matches(new));
        assert_eq!(db.note(row)?.unwrap().body,"standalone row");
        let kept: i64 = db.connection.query_row(
            "SELECT target_id FROM note_relations WHERE source_id=? AND name='Other'",[row],|r|r.get(0))?;
        assert_eq!(kept,other);
        for (page,bytes) in [(old,b"old bytes"),(new,b"new bytes")] {
            let copy = dir.join(format!("{}.txt",page.id));
            db.save_attachment_copy(db.list_attachments(page.id)?[0].id,&copy)?;
            assert_eq!(fs::read(copy)?,bytes);
        }
        drop(db);
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

pub(crate) struct SourceBatch {
    pub id: i64,
    pub title: String,
    token: String,
}

pub(crate) struct SourcePage {
    pub id: i64,
    part: String,
    path: String,
    token: String,
}

impl SourcePage {
    pub(crate) fn label(&self) -> String {
        let location = if self.part.is_empty() {
            format!("外層 ZIP／{}", self.path)
        } else {
            format!("內嵌 ZIP／{}!/{}", self.part, self.path)
        };
        format!("{location}　[#{}]", self.id)
    }
}

const SOURCE_RELATION: &str = "Notion Markdown 來源";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BoundNotionSource {
    id: i64,
    token: String,
    label: String,
}

impl BoundNotionSource {
    pub(crate) fn label(&self) -> &str { &self.label }
    pub(crate) fn matches(&self, page: &SourcePage) -> bool {
        self.id == page.id && self.token == page.token
    }
}

impl super::Notebook {
    pub(crate) fn notion_row_token(&self, id: i64) -> Result<Option<String>> {
        Ok(self.connection.query_row(
            "SELECT i.token FROM note_link_identities i JOIN notes n ON n.id=i.note_id
             WHERE n.id=? AND n.trash_root IS NULL AND EXISTS
                (SELECT 1 FROM collection_notes c WHERE c.note_id=n.id)",
            [id], |r| r.get(0),
        ).optional()?)
    }

    pub(crate) fn notion_source_batches(&self) -> Result<Vec<SourceBatch>> {
        let mut q = self.connection.prepare(
            "SELECT n.id,n.title,i.token FROM notes n JOIN note_link_identities i ON i.note_id=n.id
             WHERE n.trash_root IS NULL AND n.id IN
                (SELECT p.import_root FROM notion_page_paths p JOIN notes t ON t.id=p.note_id WHERE t.trash_root IS NULL)
             ORDER BY n.title,n.id")?;
        Ok(q.query_map([], |r| Ok(SourceBatch { id:r.get(0)?, title:r.get(1)?, token:r.get(2)? }))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn notion_source_pages(&self, batch: &SourceBatch, row: i64) -> Result<Vec<SourcePage>> {
        let mut q = self.connection.prepare(
            "SELECT p.note_id,p.archive_part,p.relative_path,i.token FROM notion_page_paths p
             JOIN notes n ON n.id=p.note_id JOIN note_link_identities i ON i.note_id=n.id
             JOIN notes root ON root.id=p.import_root JOIN note_link_identities ri ON ri.note_id=root.id
             WHERE root.id=?1 AND ri.token=?2 AND p.note_id<>?3 AND root.trash_root IS NULL AND n.trash_root IS NULL
             ORDER BY p.archive_part,p.relative_path,p.note_id")?;
        Ok(q.query_map(params![batch.id,batch.token,row], |r| Ok(SourcePage {
            id:r.get(0)?, part:r.get(1)?, path:r.get(2)?, token:r.get(3)?,
        }))?.collect::<rusqlite::Result<_>>()?)
    }

    // A user may also create edges with this name in the general relation editor.
    // Never silently replace a non-import target or choose between multiple edges.
    pub(crate) fn bound_notion_source(&self, row: i64) -> Result<Option<BoundNotionSource>> {
        let mut query = self.connection.prepare(
            "SELECT r.target_id,i.token,p.import_root,p.archive_part,p.relative_path,n.trash_root
             FROM note_relations r JOIN notes n ON n.id=r.target_id
             LEFT JOIN note_link_identities i ON i.note_id=n.id
             LEFT JOIN notion_page_paths p ON p.note_id=n.id
             WHERE r.source_id=?1 AND r.name=?2 ORDER BY r.target_id LIMIT 2")?;
        let mut edges = query.query(params![row,SOURCE_RELATION])?;
        let Some(edge) = edges.next()? else { return Ok(None); };
        let root: i64 = edge.get::<_,Option<i64>>(2)?.ok_or("現有同名關聯指向非 Notion 來源頁，請使用集合的關聯管理。")?;
        let page = SourcePage {
            id: edge.get(0)?, token: edge.get(1)?, part: edge.get(3)?, path: edge.get(4)?,
        };
        let trash = if edge.get::<_,Option<i64>>(5)?.is_some() { "（垃圾桶）" } else { "" };
        let label = format!("批次 [#{root}] {}{trash}",page.label());
        if edges.next()?.is_some() { return Err("有多個同名來源關聯，請使用集合的關聯管理確認。".into()); }
        Ok(Some(BoundNotionSource { id:page.id,token:page.token,label }))
    }

    pub(crate) fn bind_notion_source(&self, row: i64, token: &str, batch: &SourceBatch,
        page: &SourcePage, expected: Option<&BoundNotionSource>) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(&self.connection, rusqlite::TransactionBehavior::Immediate)?;
        if self.notion_row_token(row)?.as_deref() != Some(token) {
            return Err("集合資料列已變更，請重新選取。".into());
        }
        let current: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM notion_page_paths p
             JOIN notes n ON n.id=p.note_id JOIN note_link_identities i ON i.note_id=n.id
             JOIN notes root ON root.id=p.import_root JOIN note_link_identities ri ON ri.note_id=root.id
             WHERE root.id=?1 AND ri.token=?2 AND n.id=?3 AND i.token=?4 AND p.archive_part=?5 AND p.relative_path=?6
             AND root.trash_root IS NULL AND n.trash_root IS NULL AND n.id<>?7)",
            params![batch.id,batch.token,page.id,page.token,page.part,page.path,row], |r| r.get(0))?;
        if !current { return Err("來源頁的位置或身分已變更，請重新選取。".into()); }
        if self.bound_notion_source(row)?.as_ref() != expected {
            return Err("目前的來源關聯已變更，請重新選取。".into());
        }
        if let Some(old) = expected {
            if old.matches(page) { return Err("已經連結到這個 Notion Markdown 來源頁。".into()); }
            super::collections::write_relation(&tx,row,SOURCE_RELATION,old.id,false)?;
        }
        super::collections::write_relation(&tx,row,SOURCE_RELATION,page.id,true)?;
        tx.commit()?;
        Ok(())
    }
}
