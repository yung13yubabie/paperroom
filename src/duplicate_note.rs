//! Copy an approved snapshot without changing the source, native links, or external files.
use super::{MAX_TITLE_CHARS, Notebook, Result, attachments, properties, validate_body, verify_parent};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::{collections::{HashMap, HashSet}, path::Path};

const MAX_PAGES: usize = 128;
const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_COPY_ATTACHMENTS: usize = attachments::MAX_STORAGE_BYTES as usize / 2;
const MAX_PROPERTIES: usize = 128;
const MAX_PAGE_TITLE_BINDINGS: usize = 256;
const MAX_BRANCH_TITLE_BINDINGS: usize = 2048;
const STALE: &str = "原頁、子頁、屬性、連結或附件已變更。請重新預覽後再複製。";

#[derive(Eq, PartialEq)]
struct SourcePage {
    id: i64,
    parent: Option<i64>,
    title: String,
    version: i64,
    token: String,
    in_vault: bool,
}

#[derive(Eq, PartialEq)]
struct Property {
    key: String,
    value: String,
    kind: Option<String>,
}

#[derive(Eq, PartialEq)]
struct TitleBinding {
    destination: String,
    target: i64,
    target_token: String,
    target_trash: Option<i64>,
}

struct FileCopy {
    id: i64,
    name: String,
    kind: String,
    content: Vec<u8>,
}

struct PageCopy {
    source: SourcePage,
    body: String,
    properties: Vec<Property>,
    bindings: Vec<TitleBinding>,
    attachments: Vec<FileCopy>,
}

pub(crate) struct NoteDuplicate {
    include_descendants: bool,
    pages: Vec<PageCopy>,
    bytes: usize,
}

impl NoteDuplicate {
    pub(crate) fn source_title(&self) -> &str { &self.pages[0].source.title }
    pub(crate) fn pages(&self) -> usize { self.pages.len() }
    pub(crate) fn attachments(&self) -> usize { self.pages.iter().map(|page| page.attachments.len()).sum() }
    pub(crate) fn attachment_bytes(&self) -> usize { self.bytes }
}

fn read_source(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourcePage> {
    Ok(SourcePage {
        id: row.get(0)?, parent: row.get(1)?, title: row.get(2)?,
        version: row.get(3)?, token: row.get(4)?, in_vault: row.get(5)?,
    })
}

fn source_pages(db: &Connection, source: i64, include_descendants: bool) -> Result<Vec<SourcePage>> {
    let start = db.prepare_cached(
        "SELECT n.id,n.parent,n.title,n.version,i.token,
                EXISTS(SELECT 1 FROM vault_paths p WHERE p.note_id=n.id)
         FROM notes n
         JOIN note_link_identities i ON i.note_id=n.id WHERE n.id=?1 AND n.trash_root IS NULL",
    )?.query_row([source], read_source).optional()?.ok_or("找不到原頁，或原頁已在垃圾桶。")?;
    verify_parent(db, start.parent)?;
    let mut pages = vec![start];
    if !include_descendants { return Ok(pages); }
    let mut children = db.prepare_cached(
        "SELECT n.id,n.parent,n.title,n.version,i.token,
                EXISTS(SELECT 1 FROM vault_paths p WHERE p.note_id=n.id)
         FROM notes n
         LEFT JOIN note_link_identities i ON i.note_id=n.id
         WHERE n.parent=?1 AND n.trash_root IS NULL ORDER BY n.id LIMIT ?2",
    )?;
    let mut seen = HashSet::from([source]);
    let mut index = 0;
    while index < pages.len() {
        let limit = MAX_PAGES + 1 - pages.len();
        let matches = children.query_map(params![pages[index].id, limit as i64], read_source)?;
        for child in matches {
            let child = child?;
            if !seen.insert(child.id) { return Err("原分支的子頁層級含有循環。".into()); }
            if pages.len() == MAX_PAGES { return Err("一次最多可複製 128 頁，請選單頁或分支分開複製。".into()); }
            pages.push(child);
        }
        index += 1;
    }
    Ok(pages)
}

fn read_properties(db: &Connection, note: i64) -> Result<Vec<Property>> {
    let mut statement = db.prepare_cached(
        "SELECT p.key,p.value,d.kind
         FROM note_properties p LEFT JOIN property_definitions d ON d.key=p.key
         WHERE p.note_id=?1 ORDER BY p.key COLLATE BINARY LIMIT ?2",
    )?;
    let properties = statement.query_map(params![note, MAX_PROPERTIES as i64 + 1], |row| Ok(Property {
        key: row.get(0)?, value: row.get(1)?, kind: row.get(2)?,
    }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if properties.len() > MAX_PROPERTIES { return Err("原頁的屬性超過每頁 128 項上限。".into()); }
    Ok(properties)
}

fn read_title_bindings(db: &Connection, source: &SourcePage, body: &str) -> Result<Vec<TitleBinding>> {
    // The graph resolver uses these rows only for non-Vault sources.
    if source.in_vault { return Ok(Vec::new()); }
    let mut statement = db.prepare_cached(
        "SELECT b.destination,b.target_id,i.token,n.trash_root
         FROM note_title_link_bindings b LEFT JOIN notes n ON n.id=b.target_id
         LEFT JOIN note_link_identities i ON i.note_id=n.id
         WHERE b.source_id=?1 ORDER BY b.destination LIMIT ?2",
    )?;
    let mut rows = statement.query(params![source.id, MAX_PAGE_TITLE_BINDINGS as i64 + 1])?;
    let mut kept = Vec::new();
    let mut current = None;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        count += 1;
        if count > MAX_PAGE_TITLE_BINDINGS { return Err("原頁的舊標題連結超過每頁 256 筆上限。".into()); }
        let destination = row.get_ref(0)?.as_str()?;
        if destination.contains(['/', '\\', ':', '#']) || destination.len() > MAX_TITLE_CHARS * 4 { continue; }
        let keys: &mut HashSet<String> = current.get_or_insert_with(|| crate::markdown::wiki_targets(body).into_iter().collect());
        if keys.contains(destination) {
            // A retained binding to a trashed target suppresses fallback to a
            // namesake, and can resolve again if the original target is restored.
            kept.push(TitleBinding {
                destination: destination.into(), target: row.get(1)?,
                target_token: row.get(2)?, target_trash: row.get(3)?,
            });
        }
    }
    Ok(kept)
}

fn read_attachments(
    db: &Connection, note: i64, mut visit: impl FnMut(i64, &str, &str, &[u8]) -> Result<()>,
) -> Result<usize> {
    let mut statement = db.prepare_cached(
        "SELECT id,name,kind,content FROM attachments WHERE note_id=?1 ORDER BY id LIMIT ?2",
    )?;
    let mut rows = statement.query(params![note, attachments::MAX_PER_NOTE + 1])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        count += 1;
        if count > attachments::MAX_PER_NOTE as usize { return Err("原頁的附件超過每頁 16 個上限。".into()); }
        visit(row.get(0)?, row.get_ref(1)?.as_str()?, row.get_ref(2)?.as_str()?, row.get_ref(3)?.as_blob()?)?;
    }
    Ok(count)
}

fn valid_attachment(name: &str, kind: &str, content: &[u8]) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 255 && !name.chars().any(char::is_control)
        && !name.contains(['/', '\\', ':'])
        && attachments::file_kind(Path::new(name)).is_ok_and(|extension_kind| extension_kind == kind)
        && (1..=attachments::MAX_FILE_BYTES).contains(&content.len())
        && attachments::has_expected_signature(kind, content)
}

fn enforce_storage_quota(db: &Connection, bytes: usize) -> Result<()> {
    if bytes > 0 {
        let used: i64 = db.query_row("SELECT coalesce(sum(length(content)),0) FROM attachments", [], |row| row.get(0))?;
        if used > attachments::MAX_STORAGE_BYTES - bytes as i64 {
            return Err("複製附件會超過工作區 128 MiB 上限。請縮小複製範圍或先釋出附件空間。".into());
        }
    }
    Ok(())
}

impl Notebook {
    pub(crate) fn prepare_note_duplicate(&self, source: i64, include_descendants: bool) -> Result<NoteDuplicate> {
        let snapshot = self.connection.unchecked_transaction()?;
        let sources = source_pages(&snapshot, source, include_descendants)?;
        let mut pages = Vec::with_capacity(sources.len());
        let mut bytes = 0;
        let mut text_bytes = 0;
        let mut binding_count = 0;
        for source in sources {
            let body: String = snapshot.prepare_cached("SELECT body FROM notes WHERE id=?1")?
                .query_row([source.id], |row| row.get(0))?;
            validate_body(&body)?;
            let properties = read_properties(&snapshot, source.id)?;
            let bindings = read_title_bindings(&snapshot, &source, &body)?;
            binding_count += bindings.len();
            if binding_count > MAX_BRANCH_TITLE_BINDINGS {
                return Err("複製範圍的舊標題連結超過 2,048 筆，請分開複製。".into());
            }
            text_bytes += source.title.len() + body.len()
                + bindings.iter().map(|binding| binding.destination.len()).sum::<usize>();
            for property in &properties {
                if property.key.len() > properties::MAX_KEY_BYTES || property.value.len() > properties::MAX_VALUE_BYTES
                    || property.key.contains('\0') || property.value.contains('\0') {
                    return Err("原頁含有超過上限或無法複製的屬性。".into());
                }
                text_bytes += property.key.len() + property.value.len();
            }
            if text_bytes > MAX_TEXT_BYTES { return Err("複製範圍的文字、屬性與舊標題連結超過 16 MiB，請分開複製。".into()); }
            let mut files = Vec::new();
            read_attachments(&snapshot, source.id, |id, name, kind, content| {
                bytes += content.len();
                if bytes > MAX_COPY_ATTACHMENTS { return Err("複製範圍的附件超過 64 MiB，請分開複製。".into()); }
                if !valid_attachment(name, kind, content) {
                    return Err(format!("附件「{name}」的檔名、格式或內容不符，這次複製沒有執行。").into());
                }
                files.push(FileCopy { id, name: name.into(), kind: kind.into(), content: content.to_vec() });
                Ok(())
            })?;
            pages.push(PageCopy { source, body, properties, bindings, attachments: files });
        }
        enforce_storage_quota(&snapshot, bytes)?;
        snapshot.commit()?;
        Ok(NoteDuplicate { include_descendants, pages, bytes })
    }

    pub(crate) fn duplicate_note(&self, plan: &NoteDuplicate, new_title: &str) -> Result<i64> {
        let new_title = new_title.trim();
        if new_title.is_empty() || new_title.chars().count() > MAX_TITLE_CHARS || new_title.chars().any(char::is_control) {
            return Err("請輸入有效的新頁標題，最多 180 個字，請勿使用控制字元。".into());
        }
        let write = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let root = &plan.pages[0].source;
        let sources = source_pages(&write, root.id, plan.include_descendants)?;
        if !plan.pages.iter().map(|page| &page.source).eq(sources.iter()) { return Err(STALE.into()); }

        let title_key = new_title.to_lowercase();
        let mut siblings = write.prepare_cached("SELECT title FROM notes WHERE parent IS ?1 AND trash_root IS NULL")?;
        let mut rows = siblings.query([root.parent])?;
        while let Some(row) = rows.next()? {
            if row.get_ref(0)?.as_str()?.trim().to_lowercase() == title_key {
                return Err("同一層已有使用相同標題的筆記。請選擇不同的標題。".into());
            }
        }
        drop(rows);
        drop(siblings);
        enforce_storage_quota(&write, plan.bytes)?;

        let mut copies = HashMap::with_capacity(plan.pages.len());
        for page in &plan.pages {
            let source = page.source.id;
            let same_body = write.prepare_cached("SELECT 1 FROM notes WHERE id=?1 AND body=?2")?
                .exists(params![source, page.body])?;
            if !same_body || page.properties != read_properties(&write, source)?
                || page.bindings != read_title_bindings(&write, &page.source, &page.body)? {
                return Err(STALE.into());
            }
            let mut files = page.attachments.iter();
            let count = read_attachments(&write, source, |id,name,kind,content| {
                let matches = files.next().is_some_and(|old| {
                    old.id == id && old.name == name && old.kind == kind && old.content == content
                });
                if !matches { return Err(STALE.into()); }
                Ok(())
            })?;
            if count != page.attachments.len() { return Err(STALE.into()); }

            let parent = if source == root.id { root.parent } else {
                Some(*copies.get(&page.source.parent.ok_or("子頁缺少上層。")?).ok_or("找不到子頁上層的複本。")?)
            };
            let title = if source == root.id { new_title } else { &page.source.title };
            write.prepare_cached("INSERT INTO notes(parent,title,body) VALUES(?1,?2,?3)")?
                .execute(params![parent, title, page.body])?;
            let copied = write.last_insert_rowid();
            copies.insert(source, copied);
            for binding in &page.bindings {
                write.prepare_cached(
                    "INSERT INTO note_title_link_bindings(source_id,destination,target_id) VALUES(?1,?2,?3)",
                )?.execute(params![copied, binding.destination, binding.target])?;
            }
            write.prepare_cached(
                "INSERT INTO note_properties(note_id,key,value)
                 SELECT ?1,key,value FROM note_properties WHERE note_id=?2",
            )?.execute(params![copied, source])?;
            write.prepare_cached(
                "INSERT INTO attachments(note_id,name,kind,content)
                 SELECT ?1,name,kind,content FROM attachments WHERE note_id=?2 ORDER BY id",
            )?.execute(params![copied, source])?;
        }
        let copied = copies[&root.id];
        write.commit()?;
        Ok(copied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{View, note_links, typed_properties::PropertyType};

    fn token(db: &Notebook, id: i64) -> Result<String> {
        Ok(db.connection.query_row("SELECT token FROM note_link_identities WHERE note_id=?1", [id], |r| r.get(0))?)
    }

    #[test]
    fn duplicates_current_unicode_page_and_active_branch_with_independent_assets() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("duplicate.sqlite3");
        let pdf_file = dir.join("剪輯範圍.pdf");
        let text_file = dir.join("附件.txt");
        let pdf = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
        std::fs::write(&pdf_file, pdf)?;
        std::fs::write(&text_file, "甲\r\n📎")?;
        let mut db = Notebook::open(&path, false)?;
        let parent = db.add(None, "作品集", "")?;
        let original = db.add(Some(parent), "鏡頭", "")?;
        let child = db.add(Some(original), "子頁 🗂", "子頁 Unicode\r\n")?;
        let grandchild = db.add(Some(child), "孫頁", "正文\t📽")?;
        let removed = db.add(Some(original), "垃圾桶子樹", "")?;
        db.add(Some(removed), "垃圾桶孫頁", "")?;
        db.trash(removed)?;
        let child_token = token(&db, child)?;
        let target = format!("paperroom:note:{child}:{child_token}");
        let markdown = format!("# 鏡頭 🎞\r\n\r\n[子頁]({target})\r\n[相對](../原頁.md)\r\n");
        db.save(original, 1, "鏡頭", &markdown)?;
        db.pin(original)?;
        db.compare_set_property_type("時長", None, Some(PropertyType::Number))?;
        db.set_property(original, "時長", Some("003.50"))?;
        db.set_property(child, "註記", Some("原字串\r\n不折疊"))?;
        db.set_property(grandchild, "註記", Some(""))?;
        let collection = db.create_collection("原集合")?;
        db.add_to_collection(collection, original, "進行中")?;
        db.set_relation(original, "子剪輯", child, true)?;
        let first = db.import_attachment(child, &text_file)?;
        std::fs::write(&text_file, "乙\n🎥")?;
        let second = db.import_attachment(child, &text_file)?;
        let pdf_id = db.import_attachment(original, &pdf_file)?;
        db.add_pdf_annotation(pdf_id, 1, "原標註", "")?;

        let single = db.prepare_note_duplicate(original, false)?;
        assert_eq!((single.source_title(), single.pages(), single.attachments(), single.attachment_bytes()),
            ("鏡頭", 1, 1, pdf.len()));
        let one = db.duplicate_note(&single, "只複製這頁")?;
        assert!(!db.rows(View::All, "")?.iter().any(|row| row.parent == Some(one)));

        let branch = db.prepare_note_duplicate(original, true)?;
        assert_eq!((branch.pages(), branch.attachments()), (3, 3));
        let copied = db.duplicate_note(&branch, "整頁複本")?;
        let backup = dir.join("backup.sqlite3");
        db.backup(&backup)?;
        drop(db);

        let db = Notebook::open(&backup, false)?;
        let copy = db.note(copied)?.unwrap();
        assert_eq!(copy.row.parent, Some(parent));
        assert_eq!(copy.row.title, "整頁複本");
        assert_eq!(copy.body, markdown);
        assert_eq!(copy.version, 1);
        assert!(!copy.row.pinned);
        assert_ne!(token(&db, copied)?, token(&db, original)?);
        assert_eq!(db.properties(copied)?, db.properties(original)?);
        assert_eq!(db.properties(copied)?[0].1, "003.50");
        assert_eq!(db.collection_rows(collection)?[0].0.id, original);
        assert!(db.relations(copied)?.is_empty());
        assert!(db.note_revisions(copied)?.is_empty());
        assert_eq!(db.note_revisions(original)?.len(), 3);
        assert_eq!(note_links::native_note_target(&db.connection, None, &target)?, Some(child));
        let rows = db.rows(View::All, "")?;
        let children: Vec<_> = rows.iter().filter(|row| row.parent == Some(copied)).collect();
        assert_eq!(children.len(), 1);
        let copied_child = children[0].id;
        assert_eq!(children[0].title, "子頁 🗂");
        assert_ne!(copied_child, child);
        assert_ne!(token(&db, copied_child)?, child_token);
        assert_eq!(db.note(copied_child)?.unwrap().body, db.note(child)?.unwrap().body);
        assert_eq!(db.properties(copied_child)?, db.properties(child)?);
        let copied_grandchild = rows.iter().find(|row| row.parent == Some(copied_child)).unwrap().id;
        assert_eq!(db.properties(copied_grandchild)?, db.properties(grandchild)?);
        let files = db.list_attachments(copied_child)?;
        assert_eq!(files.len(), 2);
        for (file, (source_id, expected)) in files.iter().zip([(first, "甲\r\n📎"), (second, "乙\n🎥")]) {
            assert_ne!(file.id, source_id);
            assert_eq!((&*file.name, &*file.kind), ("附件.txt", "text"));
            let bytes: Vec<u8> = db.connection.query_row("SELECT content FROM attachments WHERE id=?1", [file.id], |r| r.get(0))?;
            assert_eq!(bytes, expected.as_bytes());
        }
        let copied_pdf = db.list_attachments(copied)?[0].id;
        assert_ne!(copied_pdf, pdf_id);
        assert_eq!(db.read_pdf_attachment(copied_pdf)?, ("剪輯範圍.pdf".to_owned(), pdf.to_vec()));
        assert!(db.list_pdf_annotations(copied_pdf)?.is_empty());
        assert_eq!(db.list_pdf_annotations(pdf_id)?.len(), 1);
        assert_eq!(db.note(original)?.unwrap().row.parent, Some(parent));
        assert_eq!(db.note(child)?.unwrap().row.parent, Some(original));
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn native_copies_keep_bound_renamed_targets_instead_of_namesakes() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let old = db.add(None, "舊標題", "")?;
        let source = db.add(None, "來源", "保留 [[舊標題|本來的連結]]")?;
        db.save(old, 1, "改名後", "")?;
        let reuse = db.add(None, "舊標題", "")?;
        let fresh = db.add(None, "新筆記", "保留 [[舊標題|普通連結]]")?;
        let outgoing = |db: &Notebook, id| -> Result<Vec<i64>> {
            Ok(db.related(&db.note(id)?.unwrap())?.0.into_iter().map(|row| row.id).collect())
        };
        assert_eq!(outgoing(&db, source)?, [old]);
        assert_eq!(outgoing(&db, fresh)?, [reuse]);

        let preview = db.prepare_note_duplicate(source, false)?;
        let copy = db.duplicate_note(&preview, "保留綁定")?;
        assert_eq!(outgoing(&db, copy)?, [old]);
        assert_eq!(db.note(copy)?.unwrap().body, db.note(source)?.unwrap().body);
        db.trash(old)?;
        assert!(db.duplicate_note(&preview, "狀態已變更").is_err());
        let hidden_preview = db.prepare_note_duplicate(source, false)?;
        let hidden_copy = db.duplicate_note(&hidden_preview, "保留垃圾桶目標")?;
        assert!(outgoing(&db, hidden_copy)?.is_empty());
        db.restore(old)?;
        assert_eq!(outgoing(&db, hidden_copy)?, [old]);
        assert_eq!(outgoing(&db, source)?, [old]);
        Ok(())
    }

    #[test]
    fn rejects_stale_collisions_and_rolls_back_any_partial_writes() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("duplicate.sqlite3");
        let mut db = Notebook::open(&path, false)?;
        let source = db.add(None, "來源", "")?;
        let child = db.add(Some(source), "原子頁", "")?;
        db.add(None, "Résumé", "")?;
        let text = dir.join("附件.txt");
        std::fs::write(&text, b"first")?;
        let asset = db.import_attachment(child, &text)?;
        let mut other = Notebook::open(&path, false)?;
        let snapshot = db.prepare_note_duplicate(source, true)?;
        assert!(db.duplicate_note(&snapshot, "RÉSUMÉ").is_err());
        assert!(db.duplicate_note(&snapshot, "　").is_err());
        let counts = || -> Result<(i64, i64, i64)> {
            Ok(db.connection.query_row(
                "SELECT (SELECT count(*) FROM notes),
                        (SELECT count(*) FROM note_link_identities),
                        (SELECT count(*) FROM attachments)",
                [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?)
        };

        let baseline = counts()?;
        other.set_property(child, "欄位", Some("新值"))?;
        assert!(db.duplicate_note(&snapshot, "欄位變更").is_err());
        assert_eq!(counts()?, baseline);
        let snapshot = db.prepare_note_duplicate(source, true)?;
        other.connection.execute("UPDATE attachments SET content=?2 WHERE id=?1", params![asset, b"other".as_slice()])?;
        assert!(db.duplicate_note(&snapshot, "附件同長度置換").is_err());
        assert_eq!(counts()?, baseline);
        let snapshot = db.prepare_note_duplicate(source, true)?;
        other.relocate(child, None)?;
        assert!(db.duplicate_note(&snapshot, "新層級").is_err());
        assert_eq!(counts()?, baseline);
        other.relocate(child, Some(source))?;

        let snapshot = db.prepare_note_duplicate(source, true)?;
        other.connection.execute_batch(
            "CREATE TRIGGER test_copy_abort BEFORE INSERT ON attachments
             BEGIN SELECT RAISE(ABORT, 'simulated storage failure'); END;",
        )?;
        let error = db.duplicate_note(&snapshot, "後段失敗").err().unwrap().to_string();
        assert!(error.contains("simulated storage failure"));
        assert_eq!(counts()?, baseline);
        other.connection.execute_batch("DROP TRIGGER test_copy_abort;")?;
        other.trash(source)?;
        assert!(db.duplicate_note(&snapshot, "原頁已在垃圾桶").is_err());
        assert_eq!(counts()?, baseline);
        drop(other);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
