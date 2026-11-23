use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
use std::time::Duration;

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
            std::fs::canonicalize(parent)?.join(path.file_name().ok_or("找不到筆記檔名。")?)
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
        if version > 2 {
            return Err("這份筆記由較新的紙間版本建立。".into());
        }
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 2 {
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
        let rows = self.rows(View::All, "")?;
        let mut titles: std::collections::HashMap<&str, Option<i64>> =
            std::collections::HashMap::new();
        for row in &rows {
            titles
                .entry(&row.title)
                .and_modify(|id| *id = None)
                .or_insert(Some(row.id));
        }
        let target = |key: &str| {
            key.strip_prefix('#')
                .and_then(|id| id.parse::<i64>().ok())
                .or_else(|| titles.get(key).copied().flatten())
        };
        let targets: std::collections::HashSet<_> = crate::markdown::wiki_targets(&note.body)
            .iter()
            .filter_map(|raw| target(raw))
            .collect();
        let mut query = self.connection.prepare(
            "SELECT id,body FROM notes WHERE trash_root IS NULL AND id<>?1 AND instr(body, '[[')>0",
        )?;
        let mut sources = std::collections::HashSet::new();
        let mut notes = query.query([note.row.id])?;
        while let Some(source) = notes.next()? {
            let body: String = source.get(1)?;
            if crate::markdown::wiki_targets(&body)
                .iter()
                .any(|key| target(key) == Some(note.row.id))
            {
                sources.insert(source.get::<_, i64>(0)?);
            }
        }
        let outgoing = rows
            .iter()
            .filter(|row| targets.contains(&row.id))
            .cloned()
            .collect();
        let incoming = rows
            .iter()
            .filter(|row| sources.contains(&row.id))
            .cloned()
            .collect();
        Ok((outgoing, incoming))
    }

    pub fn cards(&self) -> Result<Vec<Card>> {
        let mut query = self.connection.prepare(
            "SELECT n.id,n.title,substr(n.body,1,240),c.x,c.y
             FROM board_cards c JOIN notes n ON n.id=c.note_id
             WHERE n.trash_root IS NULL ORDER BY c.note_id",
        )?;
        let results = query
            .query_map([], |row| {
                Ok(Card {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    excerpt: row.get(2)?,
                    x: row.get(3)?,
                    y: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(results)
    }

    pub fn add_card(&mut self, id: i64) -> Result<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT count(*) >= 128 FROM board_cards WHERE note_id<>?1",
            [id],
            |row| row.get::<_, bool>(0),
        )? {
            return Err("白板最多可放 128 張卡片。".into());
        }
        tx.execute(
            "INSERT INTO board_cards(note_id,x,y)
             SELECT id, 16 + (SELECT count(*) FROM board_cards)%2*260,
                    16 + (SELECT count(*) FROM board_cards)/2*135
             FROM notes WHERE id=?1 AND trash_root IS NULL
             ON CONFLICT(note_id) DO NOTHING",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn move_card(&self, id: i64, x: i32, y: i32) -> Result<()> {
        self.connection.execute(
            "UPDATE board_cards SET x=?2,y=?3 WHERE note_id=?1",
            params![id, x.clamp(0, 10000), y.clamp(0, 50000)],
        )?;
        Ok(())
    }

    pub fn remove_card(&self, id: i64) -> Result<()> {
        self.connection
            .execute("DELETE FROM board_cards WHERE note_id=?1", [id])?;
        Ok(())
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

    pub fn save(&self, id: i64, version: i64, title: &str, body: &str) -> Result<i64> {
        validate_body(body)?;
        if title.trim().chars().count() > MAX_TITLE_CHARS {
            return Err("標題最多 180 個字。請縮短標題後再次儲存，編輯中的文字仍保留。".into());
        }
        let changed = self.connection.execute(
            "UPDATE notes SET title=?3,body=?4,version=version+1
             WHERE id=?1 AND version=?2 AND trash_root IS NULL",
            params![id, version, clean_title(title), body],
        )?;
        if changed == 0 {
            return Err(
                "這頁已由另一個視窗修改或移到垃圾桶。請先複製修改，再重新開啟本頁。".into(),
            );
        }
        Ok(version + 1)
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
        use std::collections::HashMap;

        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let root = insert(&tx, None, folder, "")?;
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
                        (crate::archive::export_title(part), "")
                    };
                    if ids.len() >= crate::archive::MAX_ENTRIES {
                        return Err("匯入產生的總頁數超過 15,000。請分批匯入。".into());
                    }
                    let id = insert(&tx, Some(parent), &title, body)?;
                    ids.insert(key, id);
                    parent = id;
                }
            }
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

    fn data_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "paperroom-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn newer_database_keeps_its_journal_mode() -> Result<()> {
        let dir = data_dir();
        let path = dir.join("future.sqlite3");
        let future = Connection::open(&path)?;
        future.pragma_update(None, "journal_mode", "DELETE")?;
        future.pragma_update(None, "user_version", 3)?;
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
        db.add_card(card)?;
        db.move_card(card, 32, 98)?;
        db.add_card(card)?;
        drop(db);
        let mut db = Notebook::open(&path, false)?;
        assert_eq!(db.cards()?.len(), 1);
        let row = db.cards()?.pop().unwrap();
        assert_eq!((row.id, row.x, row.y), (card, 32, 98));
        db.trash(card)?;
        assert!(db.cards()?.is_empty());
        db.restore(card)?;
        assert_eq!(db.cards()?[0].y, 98);
        db.remove_card(card)?;
        assert!(db.cards()?.is_empty());
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
            db.add_card(id)?;
        }
        let first = db.add(None, "甲", "")?;
        let second = db.add(None, "乙", "")?;
        let mut other = Notebook::open(&path, false)?;
        let barrier = std::sync::Barrier::new(2);
        let added = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                barrier.wait();
                other.add_card(second).is_ok()
            });
            barrier.wait();
            u8::from(db.add_card(first).is_ok()) + u8::from(worker.join().unwrap())
        });
        assert_eq!(added, 1);
        let cards = db.cards()?;
        assert_eq!(cards.len(), 128);
        db.add_card(cards[0].id)?;
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
}
