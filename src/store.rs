use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
use std::time::Duration;

#[cfg(test)]
#[path = "storyboard_acceptance.rs"]
mod storyboard_acceptance;
#[path = "note_links.rs"]
pub(crate) mod note_links;
#[path = "navigation.rs"]
pub(crate) mod navigation;
#[path = "transclusion.rs"]
pub(crate) mod transclusion;
#[path = "synced_blocks.rs"]
pub(crate) mod synced_blocks;
#[path = "html_export.rs"]
mod html_export;
#[path = "zotero.rs"]
pub(crate) mod zotero;
#[path = "mind_map.rs"]
pub(crate) mod mind_map;
#[path = "journal.rs"]
pub(crate) mod journal;
#[path = "duplicate_note.rs"]
mod duplicate_note;
#[path = "note_history.rs"]
pub(crate) mod note_history;
#[path = "note_history_state.rs"]
mod note_history_state;
#[path = "note_highlights.rs"]
pub(crate) mod note_highlights;
#[path = "board_shapes.rs"]
pub(crate) mod board_shapes;
#[path = "board_tags.rs"]
pub(crate) mod board_tags;
#[path = "board_objects.rs"]
pub(crate) mod board_objects;
#[path = "board_layout.rs"]
pub(crate) mod board_layout;
#[path = "rollups.rs"]
mod rollups;
#[path = "canvas_edit.rs"]
pub(crate) mod canvas_edit;
#[path = "canvas_bundle.rs"]
pub(crate) mod canvas_bundle;
#[path = "canvas_links.rs"]
mod canvas_links;
#[path = "vault_export.rs"]
pub(crate) mod vault_export;
#[path = "vault_zip_links.rs"]
pub(crate) mod vault_zip_links;
#[path = "vault_sync.rs"]
pub(crate) mod vault_sync;
#[path = "vault_search.rs"]
pub(crate) mod vault_search;
#[path = "workspace_search.rs"]
pub(crate) mod workspace_search;
#[path = "migration_media.rs"]
pub(crate) mod migration_media;
#[cfg(any(windows, test))]
#[path = "vault_import_review.rs"]
pub(crate) mod vault_import_review;
#[path = "notion_handoff.rs"]
pub(crate) mod notion_handoff;
#[path = "notion_page_links.rs"]
mod notion_page_links;
#[path = "notion_preview.rs"]
pub(crate) mod notion_preview;
#[path = "notion_relation_resolver.rs"]
mod notion_relation_resolver;
#[path = "snapshot_sha256.rs"]
mod snapshot_sha256;
#[path = "pdf_regions.rs"]
mod pdf_regions;
#[path = "pdf_excerpts.rs"]
mod pdf_excerpts;
#[cfg(windows)]
pub(crate) use pdf_excerpts::{
    BoardCardLibraryFilter, PdfExcerptLibraryFilter, PdfExcerptLibraryHit, PdfExcerptLibrarySource,
};
pub use pdf_regions::PdfRegion;
#[cfg(windows)]
pub(crate) use pdf_regions::PdfPageText;
#[path = "attachments.rs"]
mod attachments;
#[cfg(windows)]
pub use attachments::{MAX_PDF_SEARCH_QUERY, PdfAnnotationHit};
#[path = "collections.rs"]
mod collections;
#[cfg(any(windows, test))]
#[path = "named_relation_navigation.rs"]
pub(crate) mod named_relation_navigation;
#[cfg(windows)]
pub(crate) use collections::CollectionEntry;
#[path = "collection_display.rs"]
pub(crate) mod collection_display;
#[path = "collection_group_folds.rs"]
pub(crate) mod collection_group_folds;
#[path = "collection_field_rules.rs"]
pub(crate) mod collection_field_rules;
#[path = "collection_views.rs"]
pub(crate) mod collection_views;
#[path = "groups.rs"]
mod groups;
#[path = "workspace.rs"]
mod workspace;
#[path = "formula_service.rs"]
mod formula_service;
#[cfg(windows)]
pub use groups::BoardGroup;
#[path = "boards.rs"]
mod boards;
#[path = "tasks.rs"]
pub(crate) mod tasks;
#[path = "board_images.rs"]
pub(crate) mod board_images;
#[path = "board_bundle.rs"]
mod board_bundle;
#[path = "csv_table.rs"]
mod csv_table;
#[path = "properties.rs"]
mod properties;
#[path = "typed_properties.rs"]
pub(crate) mod typed_properties;
#[cfg(windows)]
pub use boards::BoardEdge;
pub use boards::DEFAULT_BOARD_ID;
#[cfg(any(windows, test))]
pub use properties::{MAX_KEY_BYTES, MAX_VALUE_BYTES, key_checked};

#[cfg(test)]
#[path = "integration_imports.rs"]
mod integration_imports;

const SCHEMA_VERSION: i32 = 35;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const MAX_NOTE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TITLE_CHARS: usize = 180;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    All,
    Pinned,
    Trash,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub id: i64,
    pub parent: Option<i64>,
    pub title: String,
    pub pinned: bool,
    pub trash_root: Option<i64>,
}

pub struct Note {
    pub row: Row,
    pub body: String,
    pub version: i64,
}

pub struct Card {
    pub id: i64,
    pub title: String,
    pub excerpt: String,
    pub x: i32,
    pub y: i32,
    pub image: Option<board_images::ImageSize>,
}

impl Card {
    pub(crate) fn size(&self) -> board_images::ImageSize {
        self.image.unwrap_or(board_images::ImageSize::CARD)
    }
}

pub struct Notebook {
    connection: Connection,
}

const COLS: &str = "id,parent,title,pinned,trash_root";

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: row.get(0)?,
        parent: row.get(1)?,
        title: row.get(2)?,
        pinned: row.get(3)?,
        trash_root: row.get(4)?,
    })
}

pub fn clean_title(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        return "未命名筆記".to_owned();
    }
    title.chars().take(MAX_TITLE_CHARS).collect()
}

impl Notebook {
    pub fn open(path: &Path, seed: bool) -> Result<Self> {
        // The Win32 VFS ultimately opens this UTF-16 name. Use extended
        // syntax even when Windows has not enabled long paths globally.
        #[cfg(windows)]
        let normalized = if path == Path::new(":memory:") {
            path.to_owned()
        } else {
            let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
            crate::output::windows_parent(parent)?.join(path.file_name().ok_or("找不到筆記檔名。")?)
        };
        #[cfg(windows)]
        let mut connection = Connection::open_with_flags_and_vfs(
            normalized, rusqlite::OpenFlags::default(), "win32-longpath",
        )?;
        #[cfg(not(windows))]
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "temp_store", "MEMORY")?;
        // Do not persist journal-mode changes on a newer workspace we cannot open.
        let version: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err("這份筆記由較新的紙間版本建立。".into());
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err("這份筆記由較新的紙間版本建立。".into());
        }
        if version == 0 {
            tx.execute_batch(
                "CREATE TABLE notes(
                    id INTEGER PRIMARY KEY,
                    parent INTEGER REFERENCES notes(id),
                    title TEXT NOT NULL,
                    body TEXT NOT NULL DEFAULT '',
                    pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)),
                    trash_root INTEGER REFERENCES notes(id),
                    version INTEGER NOT NULL DEFAULT 1
                 );
                 CREATE INDEX notes_by_parent ON notes(parent);
                 CREATE INDEX notes_by_trash ON notes(trash_root);
                 PRAGMA user_version=1;",
            )?;
            if seed {
                let home = insert(
                    &tx,
                    None,
                    "歡迎使用紙間",
                    "把手邊的想法留在這台電腦。用左側搜尋，也可以把相關筆記放進同一頁下方。\n\n## 從一頁開始\n\n- [ ] 建立第一篇筆記\n- [ ] 選擇「新增子頁」，整理同一主題的內容\n- [ ] 在空白行按 /，插入待辦、標題或引用\n\n## 帶走你的資料\n\n「檔案」選單提供 Markdown 匯入、匯出，以及工作區備份。\n",
                )?;
                insert(
                    &tx,
                    Some(home),
                    "讀書筆記",
                    "## 資料來源\n\n## 留下來的想法\n\n- [ ] 之後回看\n",
                )?;
                tx.execute("UPDATE notes SET pinned=1 WHERE id=?1", [home])?;
            }
        }
        if version < 2 {
            tx.execute_batch(
                "CREATE TABLE board_cards(
                    note_id INTEGER PRIMARY KEY REFERENCES notes(id),
                    x INTEGER NOT NULL CHECK(x BETWEEN 0 AND 10000),
                    y INTEGER NOT NULL CHECK(y BETWEEN 0 AND 50000)
                 );
                 PRAGMA user_version=2;",
            )?;
        }
        if version < 3 {
            tx.execute_batch(boards::SCHEMA)?;
            tx.execute_batch(properties::SCHEMA)?;
            tx.execute_batch(
                "CREATE TABLE daily_notes(
                    day TEXT PRIMARY KEY,
                    note_id INTEGER NOT NULL UNIQUE REFERENCES notes(id)
                 );
                 CREATE TABLE vault_paths(
                    note_id INTEGER PRIMARY KEY REFERENCES notes(id),
                    import_root INTEGER NOT NULL REFERENCES notes(id),
                    relative_path TEXT NOT NULL,
                    UNIQUE(import_root, relative_path)
                 );",
            )?;
        }
        if version < 4 {
            tx.execute_batch(collections::SCHEMA)?;
            tx.execute_batch(attachments::SCHEMA)?;
            tx.execute_batch(groups::SCHEMA)?;
            tx.execute_batch(workspace::SCHEMA)?;
        }
        if version < 5 {
            tx.execute_batch(canvas_links::SCHEMA)?;
            tx.execute_batch(pdf_regions::SCHEMA)?;
        }
        if version < 6 {
            tx.execute_batch(collection_views::SCHEMA)?;
        }
        if version < 7 {
            tx.execute_batch(typed_properties::SCHEMA)?;
        }
        if version < 8 {
            tx.execute_batch(groups::SOURCE_SCHEMA)?;
        }
        if version < 9 {
            tx.execute_batch(vault_sync::SCHEMA)?;
            tx.execute_batch(migration_media::SCHEMA)?;
            tx.execute_batch(pdf_regions::TEXT_MARKS_SCHEMA)?;
        }
        if version < 10 {
            tx.execute_batch(vault_sync::PUSH_SCHEMA)?;
            tx.execute_batch(pdf_regions::TEXT_SEGMENTS_SCHEMA)?;
        }
        if version < 11 {
            tx.execute_batch(collection_views::PRESENTATION_SCHEMA)?;
            tx.execute_batch(pdf_excerpts::SCHEMA)?;
        }
        if version < 12 {
            tx.execute_batch(collection_views::SCHEDULE_SCHEMA)?;
            tx.execute_batch(board_objects::SCHEMA)?;
            tx.execute_batch(note_history::SCHEMA)?;
        }
        if version < 13 {
            tx.execute_batch(collection_views::ADVANCED_SCHEMA)?;
            tx.execute_batch(board_objects::TEXT_RECOVERY_SCHEMA)?;
            tx.execute_batch(note_links::RENAME_SCHEMA)?;
            tx.execute_batch(pdf_excerpts::RECOVERY_SCHEMA)?;
            pdf_excerpts::backfill_pdf_excerpt_sources(&tx)?;
        }
        if version < 14 {
            tx.execute_batch(collection_views::TIMELINE_SCHEMA)?;
            tx.execute_batch(pdf_regions::ADJACENT_SCHEMA)?;
        }
        if version < 15 {
            tx.execute_batch(pdf_excerpts::SECOND_PAGE_SCHEMA)?;
        }
        if version < 18 {
            tx.execute_batch(collections::TEMPLATE_SCHEMA)?;
        }
        if version < 19 {
            tx.execute_batch(collections::TEMPLATE_RESOURCE_SCHEMA)?;
        }
        if version < 20 {
            tx.execute_batch(collections::TEMPLATE_CHILD_SCHEMA)?;
            tx.execute_batch(note_links::IDENTITY_SCHEMA)?;
        }
        if version < 21 {
            tx.execute_batch(board_images::SCHEMA)?;
            tx.execute_batch(collections::TEMPLATE_NESTED_SCHEMA)?;
        }
        if version < 22 {
            tx.execute_batch(board_layout::SCHEMA)?;
        }
        if version < 23 {
            tx.execute_batch(mind_map::SCHEMA)?;
            tx.execute_batch(zotero::SCHEMA)?;
        }
        if version < 24 {
            tx.execute_batch(canvas_bundle::SCHEMA)?;
        }
        // Versions 16, 17, 21 and 22 add predicates to rules_json.
        // The version guard keeps older readers from opening those formats.
        if version < 25 {
            tx.execute_batch(board_shapes::SCHEMA)?;
            tx.execute_batch(board_tags::SCHEMA)?;
        }
        if version < 26 {
            tx.execute_batch(note_highlights::SCHEMA)?;
        }
        if version < 27 {
            tx.execute_batch(workspace::CAMERA_SCHEMA)?;
        }
        if version < 28 {
            tx.execute_batch(canvas_edit::UNDO_SCHEMA)?;
        }
        if version < 29 {
            tx.execute_batch(workspace_search::SCHEMA)?;
        }
        if version < 30 {
            tx.execute_batch(collection_field_rules::SCHEMA)?;
        }
        if version < 31 {
            tx.execute_batch(notion_page_links::SCHEMA)?;
        }
        if version < 32 {
            tx.execute_batch(note_history_state::SCHEMA)?;
        }
        // Version 33 adds saved date operators to rules_json. Older readers reject their IDs.
        if version < 34 {
            tx.execute_batch(collection_display::SCHEMA)?;
        }
        if version < 35 {
            tx.execute_batch(collection_group_folds::SCHEMA)?;
        }
        if version < SCHEMA_VERSION {
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        tx.commit()?;
        Ok(Self { connection })
    }

    pub fn rows(&self, view: View, query: &str) -> Result<Vec<Row>> {
        let condition = match view {
            View::All => "trash_root IS NULL",
            View::Pinned => "trash_root IS NULL AND pinned=1",
            View::Trash => "trash_root IS NOT NULL AND (id=trash_root OR ?1 <> '')",
        };
        let sql = format!(
            "SELECT {COLS} FROM notes
             WHERE {condition} AND (
                  ?1 = '' OR instr(lower(title),lower(?1)) > 0
                  OR instr(lower(body),lower(?1)) > 0)
             ORDER BY id ASC"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let result = statement
            .query_map([query.trim()], read_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(result)
    }

    pub fn meta(&self, id: i64) -> Result<Option<Row>> {
        let sql = format!("SELECT {COLS} FROM notes WHERE id=?1");
        Ok(self.connection.query_row(&sql, [id], read_row).optional()?)
    }

    pub fn related(&self, note: &Note) -> Result<(Vec<Row>, Vec<Row>)> {
        self.indexed_related(note)
    }

    pub fn note(&self, id: i64) -> Result<Option<Note>> {
        let sql = format!("SELECT {COLS},body,version FROM notes WHERE id=?1");
        Ok(self
            .connection
            .query_row(&sql, [id], |row| {
                Ok(Note {
                    row: read_row(row)?,
                    body: row.get(5)?,
                    version: row.get(6)?,
                })
            })
            .optional()?)
    }

    pub fn add(&mut self, parent: Option<i64>, title: &str, body: &str) -> Result<i64> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_parent(&tx, parent)?;
        let id = insert(&tx, parent, title, body)?;
        tx.commit()?;
        Ok(id)
    }

    pub fn open_daily(&mut self, year: u16, month: u16, day: u16) -> Result<i64> {
        let leap = year.is_multiple_of(4)
            && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let last_day = match month {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => 0,
        };
        if !(1..=9999).contains(&year) || day == 0 || day > last_day {
            return Err("無效的日誌日期。".into());
        }
        let date = format!("{year:04}-{month:02}-{day:02}");
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = tx.query_row(
            "SELECT n.id, n.trash_root IS NOT NULL
             FROM daily_notes d JOIN notes n ON n.id=d.note_id WHERE d.day=?1",
            [&date],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
        ).optional()?;
        let id = match existing {
            Some((_, true)) => return Err("此日期的日誌在垃圾桶，請先還原。".into()),
            Some((id, false)) => id,
            None => {
                let id = insert(&tx, None, &date, "")?;
                tx.execute("INSERT INTO daily_notes(day,note_id) VALUES(?1,?2)", params![date, id])?;
                id
            }
        };
        tx.commit()?;
        Ok(id)
    }

    pub fn save(&self, id: i64, version: i64, title: &str, body: &str) -> Result<i64> {
        let tx = rusqlite::Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let saved = save_in(&tx, id, version, title, body)?;
        tx.commit()?;
        Ok(saved)
    }

    pub fn pin(&self, id: i64) -> Result<()> {
        if self.connection.execute(
            "UPDATE notes SET pinned = 1-pinned WHERE id=?1 AND trash_root IS NULL",
            [id],
        )? == 0
        {
            return Err("找不到這頁。".into());
        }
        Ok(())
    }

    pub fn relocate(&mut self, id: i64, parent: Option<i64>) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_parent(&tx, parent)?;
        let cycle: bool = tx.query_row(
            "WITH RECURSIVE descendants(id) AS (
                SELECT ?1 UNION SELECT n.id FROM notes n JOIN descendants d ON n.parent=d.id
             ) SELECT EXISTS(SELECT 1 FROM descendants WHERE id=?2)",
            params![id, parent],
            |row| row.get(0),
        )?;
        if cycle {
            return Err("無法移到這頁自己或它的子頁。".into());
        }
        if tx.execute(
            "UPDATE notes SET parent=?2 WHERE id=?1 AND trash_root IS NULL",
            params![id, parent],
        )? == 0
        {
            return Err("找不到這頁。".into());
        }
        tx.commit()?;
        Ok(())
    }

    pub fn trash(&self, id: i64) -> Result<()> {
        let count = self.connection.execute(
            "WITH RECURSIVE branch(id) AS (
                SELECT id FROM notes WHERE id=?1 AND trash_root IS NULL
                UNION ALL SELECT n.id FROM notes n JOIN branch b ON n.parent=b.id
                WHERE n.trash_root IS NULL
             )
             UPDATE notes SET trash_root=?1 WHERE id IN (SELECT id FROM branch)",
            [id],
        )?;
        if count == 0 {
            return Err("找不到這頁。".into());
        }
        Ok(())
    }

    pub fn restore(&mut self, id: i64) -> Result<i64> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let root: Option<i64> = tx
            .query_row("SELECT trash_root FROM notes WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        let root = root.ok_or("這頁已不在垃圾桶。")?;
        tx.execute(
            "UPDATE notes SET parent=NULL WHERE id=?1 AND parent IN
             (SELECT id FROM notes WHERE trash_root IS NOT NULL AND trash_root<>?1)",
            [root],
        )?;
        tx.execute(
            "UPDATE notes SET trash_root=NULL WHERE trash_root=?1",
            [root],
        )?;
        tx.commit()?;
        Ok(root)
    }

    pub fn backup(&self, path: &Path) -> Result<()> {
        crate::output::publish_new(path, |_, staging| {
            let filename = staging.to_str().ok_or("無法使用這個備份路徑。")?;
            self.connection.execute("VACUUM INTO ?1", [filename])?;
            Ok(())
        })
    }

    pub fn import(
        &mut self,
        folder: &str,
        files: Vec<crate::archive::ImportedPage>,
    ) -> Result<i64> {
        self.import_pages(folder, files, false, migration_media::MediaPlan::default(), None, None)
    }

    pub fn import_vault(
        &mut self,
        folder: &str,
        files: Vec<crate::archive::ImportedPage>,
    ) -> Result<i64> {
        self.import_pages(folder, files, true, migration_media::MediaPlan::default(), None, None)
    }

    pub fn import_vault_with_media(
        &mut self, folder: &str, files: Vec<crate::archive::ImportedPage>,
        media: migration_media::MediaPlan,
    ) -> Result<i64> {
        self.import_pages(folder, files, true, media, None, None)
    }

    pub fn import_markdown_with_media(
        &mut self, folder: &str, files: Vec<crate::archive::ImportedPage>,
        media: migration_media::MediaPlan,
    ) -> Result<i64> {
        self.import_pages(folder, files, false, media, None, None)
    }

    pub(crate) fn import_notion_with_media(
        &mut self,
        folder: &str,
        files: Vec<crate::archive::ImportedPage>,
        media: migration_media::MediaPlan,
        csv: notion_handoff::CsvSources,
    ) -> Result<i64> {
        self.import_pages(folder, files, false, media, Some(csv), None)
    }

    pub(crate) fn import_portable_vault(
        &mut self, folder: &str, import: vault_zip_links::PortableVault,
    ) -> Result<i64> {
        self.import_pages(folder, import.pages, true, import.media, None, Some(import.links))
    }

    fn import_pages(
        &mut self,
        folder: &str,
        files: Vec<crate::archive::ImportedPage>,
        vault: bool,
        media: migration_media::MediaPlan,
        csv: Option<notion_handoff::CsvSources>,
        portable_links: Option<vault_zip_links::LinkPlan>,
    ) -> Result<i64> {
        use std::collections::HashMap;

        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let root = insert(&tx, None, folder, "")?;
        let folder_title = if vault { clean_title } else { crate::archive::export_title };
        let mut ids = HashMap::new();
        for page in &files {
            validate_body(&page.body)?;
            let mut parent = root;
            for (index, part) in page.parts.iter().enumerate() {
                let key = (parent, part.as_str());
                let last = index + 1 == page.parts.len();
                if let Some(&existing) = ids.get(&key) {
                    parent = existing;
                    if last {
                        tx.execute(
                            "UPDATE notes SET title=?2,body=?3 WHERE id=?1",
                            params![existing, clean_title(&page.title), page.body],
                        )?;
                    }
                } else {
                    let (title, body) = if last {
                        (page.title.clone(), page.body.as_str())
                    } else {
                        (folder_title(part), "")
                    };
                    if ids.len() >= crate::archive::MAX_ENTRIES {
                        return Err("匯入產生的總頁數超過 15,000。請分批匯入。".into());
                    }
                    let id = insert(&tx, Some(parent), &title, body)?;
                    ids.insert(key, id);
                    parent = id;
                }
            }
            let path = page.parts.join("/");
            if vault {
                tx.execute(
                    "INSERT INTO vault_paths(note_id,import_root,relative_path) VALUES(?1,?2,?3)",
                    params![parent, root, path],
                )?;
                tx.execute(
                    "INSERT INTO vault_sync_base(note_id,body) VALUES(?1,?2)",
                    params![parent, page.body],
                )?;
                if portable_links.is_none() {
                    tx.execute("INSERT INTO vault_push_verified(note_id) VALUES(?1)", [parent])?;
                }
            }
            if let Some((part, source)) = csv.as_ref().and_then(|s| s.page_origins.get(&page.parts)) {
                tx.execute(
                    "INSERT INTO notion_page_paths(note_id,import_root,archive_part,relative_path)
                     VALUES(?1,?2,?3,?4)",
                    params![parent, root, part.as_deref().unwrap_or(""), source],
                )?;
            }
            media.attach(&tx, parent, &path)?;
        }
        if let Some(csv) = csv {
            csv.attach(&tx, root)?;
        }
        if let Some(links) = portable_links {
            links.rebind(&tx, root)?;
        }
        tx.commit()?;
        Ok(root)
    }
}

fn verify_parent(connection: &Connection, parent: Option<i64>) -> Result<()> {
    if let Some(id) = parent {
        let active: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM notes WHERE id=?1 AND trash_root IS NULL)",
            [id],
            |row| row.get(0),
        )?;
        if !active {
            return Err("找不到上層筆記。".into());
        }
    }
    Ok(())
}

fn save_in(conn: &Connection, id: i64, version: i64, title: &str, body: &str) -> Result<i64> {
    validate_body(body)?;
    if title.trim().chars().count() > MAX_TITLE_CHARS {
        return Err("標題最多 180 個字。請縮短標題後再次儲存，編輯中的文字仍保留。".into());
    }
    let previous: String = conn.query_row(
        "SELECT title FROM notes WHERE id=?1 AND version=?2 AND trash_root IS NULL",
        params![id, version], |row| row.get(0),
    ).optional()?.ok_or(
        "這頁已由另一個視窗修改或移到垃圾桶。請先複製修改，再重新開啟本頁。",
    )?;
    let title = clean_title(title);
    note_links::update_rename_bindings(conn, id, &previous, &title, body)?;
    conn.execute(
        "UPDATE notes SET title=?2,body=?3,version=version+1 WHERE id=?1",
        params![id, title, body],
    )?;
    Ok(version + 1)
}

fn validate_body(body: &str) -> Result<()> {
    if body.len() > MAX_NOTE_BYTES || body.contains('\0') {
        return Err("一篇筆記最多 4 MB UTF-8 文字，且不能包含 NUL 字元。".into());
    }
    Ok(())
}

fn insert(connection: &Connection, parent: Option<i64>, title: &str, body: &str) -> Result<i64> {
    validate_body(body)?;
    connection.execute(
        "INSERT INTO notes(parent,title,body) VALUES(?1,?2,?3)",
        params![parent, clean_title(title), body],
    )?;
    Ok(connection.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    pub(super) fn data_dir() -> std::path::PathBuf {
        loop {
            let dir = std::env::temp_dir().join(format!(
                "paperroom-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&dir) {
                Ok(()) => return dir,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test directory {dir:?}: {error}"),
            }
        }
    }

    pub(super) fn remove_v27_schema(connection: &Connection) -> Result<()> {
        remove_v28_schema(connection)?;
        connection.execute_batch("DROP TRIGGER clear_canvas_edit_undo; DROP TABLE canvas_edit_undo;")?;
        connection.execute("DROP TABLE canvas_cameras", [])?;
        Ok(())
    }

    pub(super) fn remove_v31_schema(connection: &Connection) -> Result<()> {
        connection.execute_batch(
            "DROP TRIGGER IF EXISTS reset_collection_group_folds;
             DROP TABLE IF EXISTS collection_group_folds;
             DROP TRIGGER record_prior_note_revision;
             DROP TRIGGER write_note_history;
             DROP VIEW capture_note_history;
             DROP TABLE note_revision_attachments;
             DROP TABLE note_revision_state;",
        )?;
        let trigger = note_history::SCHEMA.find("CREATE TRIGGER").unwrap();
        connection.execute_batch(&note_history::SCHEMA[trigger..])?;
        Ok(())
    }

    pub(super) fn remove_v30_schema(connection: &Connection) -> Result<()> {
        remove_v31_schema(connection)?;
        connection.execute_batch("DROP TABLE notion_page_paths;")?;
        Ok(())
    }

    pub(super) fn remove_v29_schema(connection: &Connection) -> Result<()> {
        remove_v30_schema(connection)?;
        connection.execute_batch("DROP TABLE collection_field_rules; DROP INDEX collection_notes_by_note;")?;
        Ok(())
    }

    pub(super) fn remove_v28_schema(connection: &Connection) -> Result<()> {
        remove_v29_schema(connection)?;
        connection.execute_batch(
            "DROP TRIGGER search_note_insert;
             DROP TRIGGER search_note_change;
             DROP TRIGGER search_note_delete;
             DROP TABLE workspace_note_search;",
        )?;
        Ok(())
    }

    pub(super) fn remove_v26_schema(connection: &Connection) -> Result<()> {
        remove_v27_schema(connection)?;
        connection.execute_batch("DROP TRIGGER clear_changed_note_highlights; DROP TABLE note_highlights;")?;
        Ok(())
    }

    pub(super) fn remove_v25_schema(connection: &Connection) -> Result<()> {
        remove_v26_schema(connection)?;
        connection.execute_batch("DROP TABLE board_tags; DROP TABLE board_shapes;")?;
        Ok(())
    }
    pub(super) fn remove_v24_schema(connection: &Connection) -> Result<()> {
        remove_v25_schema(connection)?;
        connection.execute_batch("DROP TABLE canvas_bundle_assets; DROP TABLE canvas_bundles;")?;
        Ok(())
    }

    pub(super) fn remove_v23_schema(connection: &Connection) -> Result<()> {
        remove_v24_schema(connection)?;
        connection.execute_batch(
            "DROP TABLE board_mind_nodes;
             DROP INDEX board_objects_board_mind_anchor;
             DROP TABLE zotero_imports;",
        )?;
        Ok(())
    }

    pub(super) fn remove_v22_schema(connection: &Connection) -> Result<()> {
        remove_v23_schema(connection)?;
        connection.execute_batch(
            "DROP TRIGGER invalidate_arranged_card_delete;
             DROP TRIGGER invalidate_arranged_card_move;
             DROP TRIGGER invalidate_arranged_object_delete;
             DROP TABLE board_layout_undo;",
        )?;
        Ok(())
    }

    pub(super) fn remove_v21_schema(connection: &Connection) -> Result<()> {
        remove_v22_schema(connection)?;
        connection.execute_batch(
            "DROP VIEW board_card_geometry;
             DROP TABLE board_image_cards;
             ALTER TABLE board_cards DROP COLUMN image_width;
             ALTER TABLE board_cards DROP COLUMN image_height;
             ALTER TABLE collection_template_children DROP COLUMN parent_ordinal;",
        )?;
        Ok(())
    }

    pub(super) fn remove_v20_schema(connection: &Connection) -> Result<()> {
        remove_v21_schema(connection)?;
        connection.execute_batch(
            "DROP TRIGGER assign_note_link_identity;
             DROP TABLE note_link_identities;
             DROP TABLE collection_template_children;",
        )?;
        Ok(())
    }

    pub(super) fn remove_v19_schema(connection: &Connection) -> Result<()> {
        remove_v20_schema(connection)?;
        connection.execute_batch("DROP TABLE collection_template_resources;")?;
        Ok(())
    }

    pub(super) fn remove_v18_schema(connection: &Connection) -> Result<()> {
        remove_v19_schema(connection)?;
        connection.execute_batch("DROP TABLE collection_templates;")?;
        Ok(())
    }

    pub(super) fn remove_v15_schema(connection: &Connection) -> Result<()> {
        remove_v18_schema(connection)?;
        connection.execute_batch(pdf_excerpts::UNDO_SECOND_PAGE_SCHEMA)?;
        Ok(())
    }

    pub(super) fn remove_v14_schema(connection: &Connection) -> Result<()> {
        remove_v15_schema(connection)?;
        connection.execute_batch(
            "DROP TABLE pdf_text_adjacent_pairs;
             ALTER TABLE collection_views DROP COLUMN timeline_mode;",
        )?;
        Ok(())
    }

    pub(super) fn remove_v13_schema(connection: &Connection) -> Result<()> {
        remove_v14_schema(connection)?;
        connection.execute_batch(
            "DROP TRIGGER delete_unreferenced_pdf_excerpt_origin;
             DROP INDEX pdf_excerpt_cards_origin;
             ALTER TABLE pdf_excerpt_cards DROP COLUMN origin_id;
             DROP TABLE pdf_excerpt_origin_regions;
             DROP TABLE pdf_excerpt_origins;
             DROP TABLE note_title_link_bindings;
             DROP TABLE board_text_recovery;
             ALTER TABLE collection_views DROP COLUMN settings_revision;
             ALTER TABLE collection_views DROP COLUMN rules_json;
             ALTER TABLE collection_views DROP COLUMN calendar_end_key;",
        )?;
        Ok(())
    }

    fn remove_v12_schema(connection: &Connection) -> Result<()> {
        remove_v13_schema(connection)?;
        connection.execute_batch(
            "DROP TRIGGER record_prior_note_revision;
             DROP TABLE note_revisions;
             DROP TABLE board_objects;
             ALTER TABLE collection_views DROP COLUMN list_mode;
             ALTER TABLE collection_views DROP COLUMN calendar_until;
             ALTER TABLE collection_views DROP COLUMN calendar_from;
             ALTER TABLE collection_views DROP COLUMN calendar_week_day;",
        )?;
        Ok(())
    }

    pub(super) fn remove_v11_schema(connection: &Connection) -> Result<()> {
        remove_v12_schema(connection)?;
        connection.execute_batch(
            "DROP TABLE pdf_excerpt_cards;
             ALTER TABLE collection_views DROP COLUMN calendar_month;
             ALTER TABLE collection_views DROP COLUMN sort_desc;
             ALTER TABLE collection_views DROP COLUMN sort_kind;
             ALTER TABLE collection_views DROP COLUMN sort_key;
             ALTER TABLE collection_views DROP COLUMN group_key;
             ALTER TABLE collection_views DROP COLUMN calendar_key;",
        )?;
        Ok(())
    }

    #[test]
    fn version_twenty_preserves_v19_template_resources_without_selecting_children() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("v19.sqlite3");
        let (template, source, target) = {
            let mut notebook = Notebook::open(&path, false)?;
            let source = notebook.add(None, "來源", "原文")?;
            let target = notebook.add(None, "目標", "原文")?;
            let child = notebook.add(Some(source), "原有子頁", "獨立來源")?;
            let collection = notebook.create_collection("舊集合")?;
            notebook.add_to_collection(collection, source, "待辦")?;
            notebook.set_relation(source, "參考", target, true)?;
            notebook.connection.execute(
                "INSERT INTO collection_templates(collection_id,name,default_title,status,body,fields_json,revision)
                 VALUES(?1,'舊資源範本','預設','待辦','保留','[]',7)", [collection],
            )?;
            let template = notebook.connection.last_insert_rowid();
            notebook.connection.execute(
                "INSERT INTO collection_template_resources(template_id,by_note,kind,target_note,name,detail,bytes)
                 VALUES(?1,?2,'relation',?3,'參考','目標',0)", params![template, source, target],
            )?;
            assert_eq!(notebook.meta(child)?.unwrap().parent, Some(source));
            remove_v20_schema(&notebook.connection)?;
            notebook.connection.pragma_update(None, "user_version", 19)?;
            (template, source, target)
        };
        {
            let notebook = Notebook::open(&path, false)?;
            let version = notebook.connection.pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))?;
            assert_eq!(version, SCHEMA_VERSION);
            let saved: (String, i64, i64, i64) = notebook.connection.query_row(
                "SELECT t.body,t.revision,r.by_note,r.target_note FROM collection_templates t
                 JOIN collection_template_resources r ON t.id=r.template_id WHERE t.id=?1", [template],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            assert_eq!(saved, ("保留".into(), 7, source, target));
            assert!(!notebook.connection.prepare("SELECT 1 FROM collection_template_children")?.exists([])?);
            assert_eq!(notebook.relations(source)?[0].1.id, target);
            assert!(!notebook.connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn version_nineteen_preserves_old_template_without_selecting_resources() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("v18.sqlite3");
        let template_id = {
            let notebook = Notebook::open(&path, false)?;
            let collection = notebook.create_collection("舊集合")?;
            notebook.connection.execute(
                "INSERT INTO collection_templates(collection_id,name,default_title,status,body,fields_json,revision)
                 VALUES(?1,'舊範本','預設','待處理',?2,'[]',4)",
                params![collection, "# 原始正文\r\n續行"],
            )?;
            let template_id = notebook.connection.last_insert_rowid();
            remove_v19_schema(&notebook.connection)?;
            notebook.connection.pragma_update(None, "user_version", 18)?;
            template_id
        };
        {
            let migrated = Notebook::open(&path, false)?;
            let version: i32 = migrated.connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            assert_eq!(version, SCHEMA_VERSION);
            let saved: (String, String, String, String, i64) = migrated.connection.query_row(
                "SELECT name, status, body, fields_json, revision FROM collection_templates WHERE id=?1",
                [template_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )?;
            assert_eq!(saved, ("舊範本".into(), "待處理".into(), "# 原始正文\r\n續行".into(), "[]".into(), 4));
            assert!(!migrated.connection.prepare("SELECT 1 FROM collection_template_resources")?.exists([])?);
            assert!(!migrated.connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn version_eighteen_adds_templates_without_rewriting_existing_rows() -> Result<()> {
        let dir = data_dir();
        let file = dir.join("v17.sqlite3");
        let (collection, note) = {
            let mut notebook = Notebook::open(&file, false)?;
            let collection = notebook.create_collection("舊集合")?;
            let note = notebook.add(None, "既有資料列", "原始內容\r\n不變")?;
            notebook.add_to_collection(collection, note, "待處理")?;
            remove_v18_schema(&notebook.connection)?;
            notebook.connection.pragma_update(None, "user_version", 17)?;
            (collection, note)
        };
        {
            let upgraded = Notebook::open(&file, false)?;
            let version: i32 = upgraded.connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            assert_eq!(version, SCHEMA_VERSION);
            assert!(!upgraded.connection.prepare("SELECT 1 FROM collection_templates")?.exists([])?);
            assert_eq!(upgraded.note(note)?.unwrap().body, "原始內容\r\n不變");
            let rows = upgraded.collection_rows(collection)?;
            assert_eq!((rows[0].0.id, rows[0].1.as_str()), (note, "待處理"));
            assert!(!upgraded.connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn version_fifteen_rolls_back_a_partial_excerpt_upgrade() -> Result<()> {
        let dir = data_dir();
        let file = dir.join("v14.sqlite3");
        let note = {
            let mut notebook = Notebook::open(&file, false)?;
            let note = notebook.add(None, "既有工作區", "原始內容")?;
            remove_v15_schema(&notebook.connection)?;
            notebook.connection.execute_batch(
                "PRAGMA user_version=14;
                 CREATE TRIGGER delete_unreferenced_pdf_excerpt_second_origin AFTER DELETE ON notes
                 BEGIN SELECT 1; END;",
            )?;
            note
        };
        assert!(Notebook::open(&file, false).is_err());
        {
            let inspect = Connection::open(&file)?;
            let version: i32 = inspect.pragma_query_value(None, "user_version", |row| row.get(0))?;
            assert_eq!(version, 14);
            let origin_trigger: String = inspect.query_row(
                "SELECT sql FROM sqlite_schema WHERE name='delete_unreferenced_pdf_excerpt_origin'",
                [], |row| row.get(0),
            )?;
            assert!(!origin_trigger.contains("pdf_excerpt_second_page"));
            assert!(!inspect.prepare(
                "SELECT 1 FROM sqlite_schema WHERE name='pdf_excerpt_second_page'",
            )?.exists([])?);
            inspect.execute_batch("DROP TRIGGER delete_unreferenced_pdf_excerpt_second_origin;")?;
        }
        {
            let upgraded = Notebook::open(&file, false)?;
            assert_eq!(upgraded.note(note)?.unwrap().body, "原始內容");
            let version: i32 = upgraded.connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            assert_eq!(version, SCHEMA_VERSION);
            assert!(!upgraded.connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn newer_database_keeps_its_journal_mode() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("future.sqlite3");
        let future = Connection::open(&path)?;
        future.pragma_update(None, "journal_mode", "DELETE")?;
        future.pragma_update(None, "user_version", SCHEMA_VERSION + 1)?;
        drop(future);
        assert!(Notebook::open(&path, false).is_err());
        let inspect = Connection::open(&path)?;
        let mode: String = inspect.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        assert_eq!(mode, "delete");
        drop(inspect);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn version_four_migration_rolls_back_on_collision() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("v3.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let note = db.add(None, "原頁", "原本文字")?;
        db.add_card_on(DEFAULT_BOARD_ID, note)?;
        db.move_card_on(DEFAULT_BOARD_ID, note, 44, 61)?;
        remove_v12_schema(&db.connection)?;
        db.connection.execute_batch(
            "DROP TABLE pdf_excerpt_cards; DROP TABLE pdf_text_mark_segments; DROP TABLE vault_push_verified; DROP TABLE pdf_text_marks; DROP TABLE vault_sync_base; DROP TABLE attachment_origins; DROP TABLE board_group_sources; DROP TABLE property_definitions; DROP TABLE collection_views;
             DROP TABLE pdf_annotation_regions; DROP TABLE canvas_vault_bindings;
             DROP TABLE collection_formulas; DROP TABLE saved_canvases;
             DROP TABLE board_group_cards; DROP TABLE board_groups;
             DROP TABLE pdf_annotations; DROP TABLE attachments;
             DROP TABLE collection_notes; DROP TABLE note_relations; DROP TABLE collections;
             PRAGMA user_version=3;
             CREATE TABLE saved_canvases(unexpected_column INTEGER);",
        )?;
        drop(db);

        assert!(Notebook::open(&path, false).is_err());
        let verify = Connection::open(&path)?;
        assert_eq!(verify.pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))?, 3);
        let leaked: i64 = verify.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name IN ('collections','attachments','board_groups')",
            [], |row| row.get(0),
        )?;
        assert_eq!(leaked, 0);
        verify.execute("DROP TABLE saved_canvases", [])?;
        drop(verify);

        let current = Notebook::open(&path, false)?;
        assert_eq!(current.note(note)?.unwrap().body, "原本文字");
        assert_eq!(current.cards_on(DEFAULT_BOARD_ID)?[0].y, 61);
        assert!(current.saved_canvases()?.is_empty());
        drop(current);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn version_five_keeps_data_and_new_views_through_backup() -> Result<()> {
        use collection_views::CollectionLayout;

        let dir = data_dir();
        let path = dir.join("v5.sqlite3");
        let copy = dir.join("backup.sqlite3");
        let source = dir.join("舊.pdf");
        std::fs::write(&source, include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf"))?;

        let mut legacy = Notebook::open(&path, false)?;
        let note = legacy.add(None, "舊分鏡", "原始正文")?;
        let collection = legacy.create_collection("分鏡")?;
        legacy.add_to_collection(collection, note, "進行中")?;
        let attachment = legacy.import_attachment(note, &source)?;
        let region = PdfRegion { x: 0.25, y: 0.25, width: 0.5, height: 0.25 };
        let annotation = legacy.add_pdf_annotation_with_region(attachment, 2, "舊引文", "舊註解", region)?;
        remove_v12_schema(&legacy.connection)?;
        legacy.connection.execute_batch("DROP TABLE pdf_excerpt_cards; DROP TABLE pdf_text_mark_segments; DROP TABLE vault_push_verified; DROP TABLE pdf_text_marks; DROP TABLE vault_sync_base; DROP TABLE attachment_origins; DROP TABLE board_group_sources; DROP TABLE property_definitions; DROP TABLE collection_views; PRAGMA user_version=5;")?;
        drop(legacy);

        let upgraded = Notebook::open(&path, false)?;
        assert!(upgraded.collection_views(collection)?.is_empty());
        assert_eq!(upgraded.collection_rows(collection)?[0].0.id, note);
        assert_eq!(upgraded.pdf_region(annotation)?, Some(region));
        let saved = upgraded.create_collection_view(collection, "目前分鏡", CollectionLayout::Table, "進行中", "分鏡")?;
        upgraded.backup(&copy)?;
        drop(upgraded);

        let restored = Notebook::open(&copy, false)?;
        assert_eq!(restored.collection_views(collection)?, [saved.clone()]);
        let rows = restored.filtered_collection_rows(collection, Some(&saved))?;
        assert_eq!(rows.iter().map(|(row, _)| row.id).collect::<Vec<_>>(), [note]);
        assert_eq!(restored.note(note)?.unwrap().body, "原始正文");
        assert_eq!(restored.pdf_region(annotation)?, Some(region));
        let version: i32 = restored.connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        assert_eq!(version, SCHEMA_VERSION);
        drop(restored);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn version_six_preserves_saved_views_and_values_and_rejects_bad_csv_atomically() -> Result<()> {
        use collection_views::CollectionLayout;
        use typed_properties::PropertyType;

        let dir = data_dir();
        let path = dir.join("v6.sqlite3");
        let mut old = Notebook::open(&path, false)?;
        let collection = old.create_collection("舊集合")?;
        let note = old.add(None, "舊列", "")?;
        old.add_to_collection(collection, note, "待處理")?;
        old.set_property(note, "工時", Some("舊文字"))?;
        let view = old.create_collection_view(collection, "舊看板", CollectionLayout::Board, "", "")?;
        remove_v11_schema(&old.connection)?;
        old.connection.execute_batch(
            "DROP TABLE pdf_text_mark_segments; DROP TABLE vault_push_verified; DROP TABLE pdf_text_marks; DROP TABLE vault_sync_base; DROP TABLE attachment_origins; DROP TABLE board_group_sources; DROP TABLE property_definitions;
             ALTER TABLE collection_views DROP COLUMN filter_key;
             ALTER TABLE collection_views DROP COLUMN filter_kind;
             ALTER TABLE collection_views DROP COLUMN filter_value;
             PRAGMA user_version=6;",
        )?;
        drop(old);

        let mut upgraded = Notebook::open(&path, false)?;
        assert_eq!(upgraded.collection_views(collection)?, [view]);
        upgraded.compare_set_property_type("工時", None, Some(PropertyType::Number))?;
        assert_eq!(upgraded.properties(note)?, [("工時".into(), "舊文字".into())]);
        let count = upgraded.rows(View::All, "")?.len();
        assert!(upgraded.import_csv("不完整匯入", "標題,工時\n有效,012.50\n無效,壞值\n".as_bytes()).is_err());
        assert_eq!(upgraded.rows(View::All, "")?.len(), count);
        let (_, count) = upgraded.import_csv("完整匯入", "標題,工時\n有效,012.50\n".as_bytes())?;
        assert_eq!(count, 1);
        assert_eq!(upgraded.properties(upgraded.rows(View::All, "有效")?[0].id)?.iter()
                   .find(|(key, _)| key == "工時").unwrap().1, "012.50");
        drop(upgraded);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn vault_links_resolve_by_source_path_without_rewriting_notes() -> Result<()> {
        use crate::archive::ImportedPage;
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let local = db.add(None, "Outside", "")?;
        let text = format!("[[../B/Target#header|target]] [[Target]] [[#{local}]]");
        let source = [("A", "Source.md", text.as_str()), ("A", "Target.md", ""),
                      ("B", "Target.md", "second")].into_iter().map(|(folder, file, body)| ImportedPage {
            parts: vec![folder.into(), file.into()],
            title: file.trim_end_matches(".md").into(),
            body: body.into(),
        }).collect();
        db.import_vault("Vault", source)?;
        let source_id = db.rows(View::All, "Source")?[0].id;
        let note = db.note(source_id)?.unwrap();
        let (targets, _) = db.related(&note)?;
        assert_eq!(note.body, text);
        assert_eq!(targets.len(), 1);
        let target = db.note(targets[0].id)?.unwrap();
        assert_eq!(target.body, "second");
        assert_eq!(db.related(&target)?.1[0].id, source_id);
        let version = note.version;
        db.save(source_id, version, "Source", &format!("[[paperroom:note:{local}|Outside]]"))?;
        assert_eq!(db.related(&db.note(source_id)?.unwrap())?.0[0].id, local);
        Ok(())
    }

    #[test]
    fn version_nine_refreshes_only_provable_vault_baselines_and_keeps_old_pdf_marks() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("v9.sqlite3");
        let vault = dir.join("Vault");
        std::fs::create_dir(&vault)?;
        for file in ["unchanged.md", "unproven.md"] {
            std::fs::write(vault.join(file), "original")?;
        }
        let pdf = dir.join("old.pdf");
        std::fs::write(&pdf, b"%PDF-2.0\n")?;
        let region = PdfRegion { x: 0.15, y: 0.3, width: 0.25, height: 0.04 };
        let (unproven, annotation) = {
            let mut workspace = Notebook::open(&path, false)?;
            let root = workspace.import_vault("Vault", crate::vault::read(&vault)?.pages)?;
            let unproven = workspace.connection.query_row(
                "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path='unproven.md'",
                [root], |r| r.get(0),
            )?;
            let note = workspace.note(unproven)?.unwrap();
            workspace.save(unproven, note.version, &note.row.title, "paperroom edit")?;
            let attachment = workspace.import_attachment(root, &pdf)?;
            let annotation = workspace.add_pdf_annotation_with_region(
                attachment, 1, "old quote", "", region,
            )?;
            workspace.connection.execute("INSERT INTO pdf_text_marks(annotation_id) VALUES(?1)", [annotation])?;
            remove_v11_schema(&workspace.connection)?;
            workspace.connection.execute_batch(
                "DROP TABLE pdf_text_mark_segments;
                 DROP TABLE vault_push_verified;
                 PRAGMA user_version=9;",
            )?;
            (unproven, annotation)
        };
        for _ in 0..2 {
            let workspace = Notebook::open(&path, false)?;
            let connection = &workspace.connection;
            let version: i32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
            assert_eq!(version, SCHEMA_VERSION);
            let proven = connection.query_row(
                "SELECT p.relative_path FROM vault_push_verified v
                 JOIN vault_paths p ON p.note_id=v.note_id", [], |r| r.get::<_, String>(0),
            )?;
            let count: i64 = connection.query_row("SELECT COUNT(*) FROM vault_push_verified", [], |r| r.get(0))?;
            assert_eq!((proven.as_str(), count), ("unchanged.md", 1));
            assert_eq!(workspace.note(unproven)?.unwrap().body, "paperroom edit");
            assert_eq!(workspace.pdf_region(annotation)?, Some(region));
            assert_eq!(connection.query_row("SELECT annotation_id FROM pdf_text_marks", [], |r| r.get::<_, i64>(0))?, annotation);
            let segments: i64 = connection.query_row("SELECT COUNT(*) FROM pdf_text_mark_segments", [], |r| r.get(0))?;
            assert_eq!(segments, 0);
            assert!(!connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn version_eleven_rolls_back_failed_upgrade_and_reopens_legacy_views() -> Result<()> {
        use collection_views::CollectionLayout;

        let dir = data_dir();
        let file = dir.join("v10.sqlite3");
        let (collection, note, old_view) = {
            let mut notebook = Notebook::open(&file, false)?;
            let collection = notebook.create_collection("分鏡")?;
            let note = notebook.add(None, "原始資料列", "舊正文")?;
            notebook.add_to_collection(collection, note, "待處理")?;
            let old_view = notebook.create_collection_view(collection, "舊表格", CollectionLayout::Table, "", "")?;
            remove_v11_schema(&notebook.connection)?;
            notebook.connection.execute_batch(
                "PRAGMA user_version=10;
                 CREATE TABLE pdf_excerpt_cards(colliding_column TEXT);",
            )?;
            (collection, note, old_view)
        };
        assert!(Notebook::open(&file, false).is_err());
        let inspect = Connection::open(&file)?;
        let version: i32 = inspect.pragma_query_value(None, "user_version", |row| row.get(0))?;
        assert_eq!(version, 10);
        let columns: i64 = inspect.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('collection_views')
             WHERE name IN ('calendar_key','calendar_month','group_key','sort_key','sort_kind','sort_desc')",
            [], |row| row.get(0),
        )?;
        assert_eq!(columns, 0);
        inspect.execute("DROP TABLE pdf_excerpt_cards", [])?;
        drop(inspect);

        let notebook = Notebook::open(&file, false)?;
        assert_eq!(notebook.collection_views(collection)?, [old_view]);
        assert_eq!(notebook.collection_rows(collection)?[0].0.id, note);
        assert_eq!(notebook.note(note)?.unwrap().body, "舊正文");
        let version: i32 = notebook.connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        assert_eq!(version, SCHEMA_VERSION);
        assert!(!notebook.connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
        drop(notebook);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn opens_a_version_two_workspace_and_reopens_its_new_backup() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("legacy.sqlite3");
        let legacy = Connection::open(&path)?;
        legacy.execute_batch(
            "CREATE TABLE notes(
                id INTEGER PRIMARY KEY, parent INTEGER REFERENCES notes(id),
                title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '',
                pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)),
                trash_root INTEGER REFERENCES notes(id), version INTEGER NOT NULL DEFAULT 1
             );
             CREATE INDEX notes_by_parent ON notes(parent);
             CREATE INDEX notes_by_trash ON notes(trash_root);
             CREATE TABLE board_cards(
                note_id INTEGER PRIMARY KEY REFERENCES notes(id),
                x INTEGER NOT NULL CHECK(x BETWEEN 0 AND 10000),
                y INTEGER NOT NULL CHECK(y BETWEEN 0 AND 50000)
             );
             INSERT INTO notes(id,title,body) VALUES(42, '舊筆記', '原始正文');
             INSERT INTO board_cards VALUES(42, 123, 234);
             PRAGMA user_version=2;"
        )?;
        drop(legacy);

        let mut upgraded = Notebook::open(&path, false)?;
        assert_eq!(upgraded.note(42)?.unwrap().body, "原始正文");
        assert_eq!((upgraded.cards_on(DEFAULT_BOARD_ID)?[0].x,
                    upgraded.cards_on(DEFAULT_BOARD_ID)?[0].y), (123, 234));
        let second = upgraded.create_board("另一張白板")?;
        upgraded.add_card_on(second, 42)?;
        upgraded.move_card_on(second, 42, 12, 39)?;
        upgraded.compare_set_property(42, "階段", None, Some("進行中"))?;
        let copy = dir.join("copy.sqlite3");
        upgraded.backup(&copy)?;
        drop(upgraded);

        let reopened = Notebook::open(&copy, false)?;
        assert_eq!(reopened.cards_on(DEFAULT_BOARD_ID)?[0].y, 234);
        assert_eq!(reopened.cards_on(second)?[0].y, 39);
        assert_eq!(reopened.properties(42)?, [("階段".to_owned(), "進行中".to_owned())]);
        let integrity: String = reopened.connection.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
        assert_eq!(integrity, "ok");
        assert_eq!(reopened.connection.pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))?, SCHEMA_VERSION);
        drop(reopened);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn daily_note_identity_survives_edits_trash_and_backup() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("daily.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        assert!(db.open_daily(2025, 2, 29).is_err());
        let id = db.open_daily(2024, 2, 29)?;
        let note = db.note(id)?.unwrap();
        assert_eq!(note.row.title, "2024-02-29");
        db.save(id, note.version, "閏日", "留存內容")?;
        let mut concurrent = Notebook::open(&path, false)?;
        assert_eq!(concurrent.open_daily(2024, 2, 29)?, id);
        db.trash(id)?;
        assert!(concurrent.open_daily(2024, 2, 29).is_err());
        db.restore(id)?;
        assert_eq!(concurrent.open_daily(2024, 2, 29)?, id);
        drop(concurrent);
        let backup = dir.join("backup.sqlite3");
        db.backup(&backup)?;
        let mut copy = Notebook::open(&backup, false)?;
        assert_eq!(copy.open_daily(2024, 2, 29)?, id);
        assert_eq!(copy.note(id)?.unwrap().body, "留存內容");
        drop(copy);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn persists_chinese_search_and_rejects_stale_edit() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("notes.sqlite3");
        let mut notebook = Notebook::open(&path, false)?;
        let id = notebook.add(None, "中文筆記", "# 越野\n\n- [x] 補給")?;
        assert_eq!(notebook.rows(View::All, "補給")?[0].id, id);
        let second = Notebook::open(&path, false)?;
        assert_eq!(notebook.save(id, 1, "中文筆記", "新內容")?, 2);
        assert!(second.save(id, 1, "舊版本", "舊內容").is_err());
        drop(second);
        drop(notebook);
        let reopened = Notebook::open(&path, false)?;
        assert_eq!(reopened.note(id)?.unwrap().body, "新內容");
        drop(reopened);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn incoming_and_outgoing_links_survive_renames_and_ignore_literal_code() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let target = db.add(None, "連結目標", "")?;
        let source = db.add(None, "研究筆記", "[[連結目標|縮寫]]")?;
        let via_id = db.add(None, "固定連結", &format!("[[#{target}|自己的別名]]"))?;
        db.add(None, "程式碼", "`[[連結目標]]`")?;
        let links = db.related(&db.note(target)?.unwrap())?.1;
        assert_eq!(
            links.iter().map(|row| row.id).collect::<Vec<_>>(),
            [source, via_id]
        );
        assert_eq!(db.related(&db.note(source)?.unwrap())?.0[0].id, target);
        let duplicate = db.add(None, "連結目標", "")?;
        assert!(db.related(&db.note(source)?.unwrap())?.0.is_empty());
        db.save(target, 1, "修訂後標題", "")?;
        assert_eq!(db.related(&db.note(via_id)?.unwrap())?.0[0].id, target);
        assert_eq!(db.related(&db.note(source)?.unwrap())?.0[0].id, duplicate);
        db.trash(target)?;
        assert!(db.related(&db.note(via_id)?.unwrap())?.0.is_empty());
        Ok(())
    }

    #[test]
    fn card_positions_persist_and_trash_does_not_destroy_board_layout() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("cards.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let card = db.add(None, "卡片", "第一行")?;
        db.add_card_on(DEFAULT_BOARD_ID, card)?;
        db.move_card_on(DEFAULT_BOARD_ID, card, 32, 98)?;
        db.add_card_on(DEFAULT_BOARD_ID, card)?;
        drop(db);
        let mut db = Notebook::open(&path, false)?;
        assert_eq!(db.cards_on(DEFAULT_BOARD_ID)?.len(), 1);
        let row = db.cards_on(DEFAULT_BOARD_ID)?.pop().unwrap();
        assert_eq!((row.id, row.x, row.y), (card, 32, 98));
        db.trash(card)?;
        assert!(db.cards_on(DEFAULT_BOARD_ID)?.is_empty());
        db.restore(card)?;
        assert_eq!(db.cards_on(DEFAULT_BOARD_ID)?[0].y, 98);
        db.remove_card_on(DEFAULT_BOARD_ID, card)?;
        assert!(db.cards_on(DEFAULT_BOARD_ID)?.is_empty());
        assert!(db.note(card)?.is_some());
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn concurrent_card_additions_respect_the_shared_limit() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("cards.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        for _ in 0..127 {
            let id = db.add(None, "卡片", "")?;
            db.add_card_on(DEFAULT_BOARD_ID, id)?;
        }
        let first = db.add(None, "甲", "")?;
        let second = db.add(None, "乙", "")?;
        let mut other = Notebook::open(&path, false)?;
        let barrier = std::sync::Barrier::new(2);
        let added = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                barrier.wait();
                other.add_card_on(DEFAULT_BOARD_ID, second).is_ok()
            });
            barrier.wait();
            u8::from(db.add_card_on(DEFAULT_BOARD_ID, first).is_ok()) + u8::from(worker.join().unwrap())
        });
        assert_eq!(added, 1);
        let cards = db.cards_on(DEFAULT_BOARD_ID)?;
        assert_eq!(cards.len(), 128);
        db.add_card_on(DEFAULT_BOARD_ID, cards[0].id)?;
        drop(other);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn deep_unicode_destination_preserves_export_and_backup() -> Result<()> {
        let root = data_dir();
        let mut dir = root.clone();
        for _ in 0..12 {
            dir.push("深層資料夾-01234567890123456789");
        }
        std::fs::create_dir_all(&dir)?;
        let path = root.join("notes.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let id = db.add(None, "中文標題", "保留內文")?;
        let text = dir.join("作品.md");
        crate::archive::export_note(&text, "中文標題", "保留內文")?;
        assert_eq!(crate::archive::markdown(&text)?.body, "保留內文");
        let path = dir.join("筆記備份.sqlite3");
        db.backup(&path)?;
        let restored = Notebook::open(&path, false)?;
        assert_eq!(restored.note(id)?.unwrap().body, "保留內文");
        drop(restored);
        drop(db);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn nested_trash_restore_move_pin_and_backup() -> Result<()> {
        let dir = data_dir();
        let mut notebook = Notebook::open(&dir.join("notes.sqlite3"), false)?;
        let top = notebook.add(None, "研究", "")?;
        let kid = notebook.add(Some(top), "資料", "")?;
        let sibling = notebook.add(Some(top), "草稿", "")?;
        assert!(notebook.relocate(top, Some(kid)).is_err());
        notebook.pin(kid)?;
        assert_eq!(notebook.rows(View::Pinned, "")?[0].id, kid);
        notebook.trash(sibling)?;
        notebook.trash(top)?;
        assert_eq!(notebook.rows(View::Trash, "")?.len(), 2);
        assert!(notebook.rows(View::All, "")?.is_empty());
        assert_eq!(notebook.restore(kid)?, top);
        assert_eq!(notebook.note(kid)?.unwrap().row.parent, Some(top));
        assert_eq!(notebook.rows(View::Trash, "")?[0].id, sibling);
        notebook.relocate(kid, None)?;
        let path = dir.join("backup.sqlite3");
        notebook.backup(&path)?;
        assert!(notebook.backup(&path).is_err());
        let restored = Notebook::open(&path, false)?;
        assert_eq!(restored.note(kid)?.unwrap().row.parent, None);
        assert!(restored.note(kid)?.unwrap().row.pinned);
        assert_eq!(
            restored.note(sibling)?.unwrap().row.trash_root,
            Some(sibling)
        );
        drop(restored);
        drop(notebook);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
    #[test]
    fn live_wal_backup_reopens_as_an_independent_portable_workspace() -> Result<()> {
        use super::board_shapes::{ShapeKind, ShapeRect};
        use std::fs;

        let root = data_dir();
        let source = root.join("使用中.sqlite3");
        let archive = root.join("紙間備份.sqlite3");
        let fixture = root.join("剪輯範圍.pdf");
        let expected = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
        fs::write(&fixture, expected)?;
        let mut live = Notebook::open(&source, false)?;
        // Leave all new commits in WAL and capture the still-open workspace.
        live.connection.pragma_update(None, "wal_autocheckpoint", 0)?;
        live.connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        let note = live.add(None, "原始標題", "備份快照")?;
        let sibling = live.add(None, "第二張卡", "連線端點")?;
        let attachment = live.import_attachment(note, &fixture)?;
        let board = live.create_board("研究白板")?;
        live.add_card_on(board, note)?;
        live.add_card_on(board, sibling)?;
        live.move_card_on(board, note, 192, 360)?;
        live.add_edge_on(board, note, sibling)?;
        let rect = ShapeRect { x: 480, y: 224, width: 192, height: 96 };
        let shape = live.create_board_shape(board, ShapeKind::Ellipse, rect)?;
        live.set_board_tag(board, "回放", true)?;
        assert!(fs::metadata(root.join("使用中.sqlite3-wal"))?.len() > 32);
        live.backup(&archive)?;
        let saved = fs::read(&archive)?;
        assert!(saved.starts_with(b"SQLite format 3\0"));
        assert!(live.backup(&archive).is_err());
        assert_eq!(fs::read(&archive)?, saved);

        // A portable restore is one file, with no reliance on the live DB,
        // its WAL, the previously imported attachment, or a socket.
        let portable = root.join("獨立工作區").join("paperroom-data");
        fs::create_dir_all(&portable)?;
        let copy_path = portable.join("notes.sqlite3");
        fs::copy(&archive, &copy_path)?;
        assert_eq!(fs::read_dir(&portable)?.count(), 1);
        let current = live.note(note)?.unwrap();
        live.save(note, current.version, "現在", "備份之後")?;
        live.remove_attachment(attachment)?;
        live.remove_edge_on(board, note, sibling)?;
        let current_shape = live.board_shapes(board)?.into_iter().next().unwrap();
        live.delete_board_shape(board, shape, current_shape.version)?;
        assert_eq!(live.note(note)?.unwrap().body, "備份之後");
        assert!(live.list_attachments(note)?.is_empty());
        assert!(live.edges_on(board)?.is_empty());
        assert!(live.board_shapes(board)?.is_empty());
        fs::remove_file(&fixture)?;

        {
            let replica = Notebook::open(&copy_path, true)?;
            assert!(!replica.connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
            assert_eq!(replica.note(note)?.unwrap().body, "備份快照");
            assert_eq!(replica.list_attachments(note)?[0].id, attachment);
            let materialized = portable.join("回復的附件.pdf");
            replica.save_attachment_copy(attachment, &materialized)?;
            assert_eq!(fs::read(&materialized)?, expected);
            let cards = replica.cards_on(board)?;
            assert_eq!(cards.len(), 2);
            assert!(cards.iter().any(|card| (card.id, card.x, card.y) == (note, 192, 360)));
            let edges = replica.edges_on(board)?;
            assert_eq!(edges.len(), 1);
            assert_eq!((edges[0].from, edges[0].to), (note, sibling));
            let shapes = replica.board_shapes(board)?;
            assert_eq!(shapes.len(), 1);
            assert_eq!((shapes[0].id, shapes[0].kind, shapes[0].rect), (shape, ShapeKind::Ellipse, rect));
            assert_eq!(replica.board_tags(board)?, ["回放"]);
        }
        assert_eq!(fs::read(&archive)?, saved);
        drop(live);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn old_board_workspace_upgrades_atomically_and_backup_restores_typed_objects() -> Result<()> {
        use super::board_shapes::{ShapeKind, ShapeRect};
        use super::board_tags::TagSubject;
        let dir = data_dir();
        let path = dir.join("legacy.sqlite3");
        let backup = dir.join("backup.sqlite3");
        let (note, board) = {
            let mut db = Notebook::open(&path, false)?;
            let note = db.add(None, "研究板", "#觀察")?;
            let board = DEFAULT_BOARD_ID;
            db.connection.execute("UPDATE boards SET title='研究板' WHERE id=?1", [board])?;
            db.add_card_on(board, note)?;
            remove_v25_schema(&db.connection)?;
            db.connection.pragma_update(None, "user_version", 24)?;
            // Force the second CREATE TABLE to fail after the first, proving the migration is atomic.
            db.connection.execute_batch("CREATE TABLE board_tags(deliberate_conflict INTEGER);")?;
            (note, board)
        };
        assert!(Notebook::open(&path, false).is_err());
        {
            let raw = Connection::open(&path)?;
            assert_eq!(raw.pragma_query_value(None,"user_version",|r|r.get::<_,i32>(0))?,24);
            assert!(!raw.prepare("SELECT 1 FROM sqlite_schema WHERE name='board_shapes'")?.exists([])?);
            assert_eq!(raw.query_row("SELECT note_id FROM board_cards WHERE board_id=?1",[board],|r|r.get::<_,i64>(0))?,note);
            raw.execute_batch("DROP TABLE board_tags")?;
        }
        let db = Notebook::open(&path, false)?;
        assert_eq!(db.note(note)?.unwrap().body, "#觀察");
        assert_eq!(db.cards_on(board)?.len(), 1);
        assert!(db.board_shapes(board)?.is_empty());
        assert!(db.board_tags(board)?.is_empty());
        let rect = ShapeRect { x:520,y:140,width:140,height:80 };
        let shape = db.create_board_shape(board,ShapeKind::Ellipse,rect)?;
        db.set_board_tag(board,"觀察",true)?;
        db.backup(&backup)?;
        drop(db);
        let copy=Notebook::open(&backup,false)?;
        assert!(!copy.connection.prepare("PRAGMA foreign_key_check")?.exists([])?);
        let shapes=copy.board_shapes(board)?;
        assert_eq!(shapes.len(),1);
        assert_eq!((shapes[0].id,shapes[0].rect,shapes[0].kind),(shape,rect,ShapeKind::Ellipse));
        let hits=copy.exact_board_and_note_tag_hits("觀察",&[(note,"研究板".to_owned())])?;
        assert_eq!(hits.iter().map(|h|h.subject).collect::<Vec<_>>(),vec![TagSubject::Board(board),TagSubject::Note(note)]);
        assert_eq!(copy.cards_on(board)?.len(),1);
        Ok(())
    }

}
