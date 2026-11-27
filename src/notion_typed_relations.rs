//! Explicit local handoff. The manifest is caller-supplied, not proof of Notion origin.
use super::Result;
use rusqlite::{Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const MEMBER: &str = "paperroom-transfer/typed-relations-v1.json";
pub(crate) const LIMIT: u64 = 1024 * 1024;
type Edge = (String, String, String);

pub(crate) struct Plan {
    pages: BTreeSet<String>,
    edges: BTreeSet<Edge>,
}

fn canonical(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Plan {
    pub(crate) fn read(bytes: &[u8]) -> Result<Self> {
        let (schema, memberships, relations): (String, Vec<(String, String)>, Vec<(String, String, String, String, String)>) =
            serde_json::from_slice(bytes)?;
        if schema != "paperroom-notion-typed-relations/1" || !(1..=128).contains(&memberships.len()) || relations.len() > 2048 {
            return Err("具名關聯交接版本或數量不符。".into());
        }
        let mut pages = BTreeMap::new();
        for (page, source) in memberships {
            if !canonical(&page) || !canonical(&source) || pages.insert(page, source).is_some() {
                return Err("交接頁面身分或資料來源重複或無效。".into());
            }
        }
        let mut edges = BTreeSet::new();
        let mut declarations = BTreeMap::new();
        for (from, label, kind, destination_source, to) in relations {
            let source = pages.get(&from).ok_or("具名關聯的來源頁不在確認範圍。")?;
            if from == to || kind != "relation" || pages.get(&to) != Some(&destination_source)
                || label.is_empty() || label.trim() != label || label.len() > 256 || label.chars().any(char::is_control) {
                return Err("關聯的欄位、型別、目標頁或資料來源不符。".into());
            }
            if declarations.insert((source.clone(), label.clone()), destination_source.clone())
                    .is_some_and(|previous| previous != destination_source)
                || !edges.insert((from, label, to)) {
                return Err("具名關聯重複，或同一來源欄位的目標資料庫不一致。".into());
            }
        }
        Ok(Self { pages: pages.into_keys().collect(), edges })
    }

    pub(crate) fn page_count(&self) -> usize { self.pages.len() }

    pub(crate) fn count(&self) -> usize { self.edges.len() }

    pub(crate) fn attach(&self, tx: &Transaction<'_>, root: i64) -> Result<()> {
        let mut local = BTreeMap::new();
        let mut query = tx.prepare_cached("SELECT note_id FROM notion_page_paths
            WHERE import_root=?1 AND archive_part='' AND relative_path=?2")?;
        for id in &self.pages {
            let note: i64 = query.query_row(params![root, format!("Page {id}.md")], |row| row.get(0))
                .map_err(|_| "找不到本批外層 ZIP 中精確對應的 Markdown 頁面。")?;
            local.insert(id, note);
        }
        for (from, name, to) in &self.edges {
            super::collections::write_relation(tx, local[from], name, local[to], true)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::notion_archive_with_media;
    use crate::store::Notebook;
    use std::{fs, io::Write, path::Path, time::{SystemTime, UNIX_EPOCH}};

    fn zip(path: &Path, pages: &[(&str, &str)], sidecar: &[u8]) -> Result<()> {
        let mut writer = zip::ZipWriter::new(fs::File::create(path)?);
        let options = zip::write::SimpleFileOptions::default();
        for (id, body) in pages {
            writer.start_file(format!("Page {id}.md"), options)?;
            writer.write_all(format!("# 同名\n\n{body}").as_bytes())?;
        }
        writer.start_file("paperroom-transfer/manifest.json", options)?;
        writer.write_all(b"{}")?;
        writer.start_file(MEMBER, options)?;
        writer.write_all(sidecar)?;
        writer.finish()?;
        Ok(())
    }

    fn run_import(db: &mut Notebook, zip: &Path) -> Result<i64> {
        let (pages, media, sources) = notion_archive_with_media(zip)?;
        db.import_notion_with_media("測試", pages.pages, media, sources)
    }

    #[test]
    fn invalid_source_schema_and_ambiguous_edges_are_rejected() {
        let (a, b, ds) = ("1".repeat(32), "2".repeat(32), "d".repeat(32));
        let valid = serde_json::json!([a,"取景","relation",ds,b]);
        let cases = [
            serde_json::json!([[a,"取景","url",ds,b]]),
            serde_json::json!([[a,"取景","relation","e".repeat(32),b]]),
            serde_json::json!([[a,"取景","relation",ds,a]]),
            serde_json::json!([valid, valid]),
        ];
        for edges in cases {
            let bytes = serde_json::to_vec(&serde_json::json!([
                "paperroom-notion-typed-relations/1", [[a,ds],[b,ds]], edges
            ])).unwrap();
            assert!(Plan::read(&bytes).is_err());
        }
    }

    #[test]
    fn named_edges_resolve_only_exact_members_of_this_import_and_rollback_otherwise() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-typed-{}-{}", std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        fs::create_dir(&dir)?;
        let run = || -> Result<()> {
            let (a, b, c, ds) = ("1".repeat(32), "2".repeat(32), "3".repeat(32), "d".repeat(32));
            let mut db = Notebook::open(&dir.join("live.sqlite3"), false)?;
            let payload = serde_json::to_vec(&serde_json::json!([
                "paperroom-notion-typed-relations/1", [[a,ds],[b,ds],[c,ds]], [[a,"取景","relation",ds,b]]
            ]))?;
            let archive = dir.join("local.zip");
            zip(&archive, &[(&a, "來源"), (&b,"甲"), (&c,"乙")], &payload)?;
            let root = run_import(&mut db, &archive)?;
            let source = db.connection.query_row("SELECT note_id FROM notion_page_paths WHERE import_root=?1 AND relative_path=?2",
                params![root, format!("Page {a}.md")], |r| r.get::<_,i64>(0))?;
            let second = run_import(&mut db, &archive)?;
            assert_ne!(second, root);
            let before: i64 = db.connection.query_row("SELECT count(*) FROM notes", [], |r| r.get(0))?;
            let d = "4".repeat(32);
            zip(&archive, &[(&a,"另個來源"), (&c,"乙"), (&d,"甲")], &payload)?;
            assert!(run_import(&mut db, &archive).is_err());
            assert_eq!(db.connection.query_row("SELECT count(*) FROM notes", [], |r| r.get::<_,i64>(0))?, before);
            assert_eq!(db.connection.query_row("SELECT count(*) FROM note_relations", [], |r| r.get::<_,i64>(0))?, 2);
            drop(db);
            let db = Notebook::open(&dir.join("live.sqlite3"), false)?;
            db.backup(&dir.join("copy.sqlite3"))?;
            drop(db);
            let db = Notebook::open(&dir.join("copy.sqlite3"), false)?;
            let related = db.relations(source)?;
            assert_eq!(related.len(), 1);
            assert_eq!(related[0].0, "取景");
            let target = db.note(related[0].1.id)?.unwrap();
            assert_eq!(target.body,"甲");
            assert_eq!(target.row.parent, Some(root));
            Ok(())
        };
        let result = run();
        fs::remove_dir_all(dir)?;
        result
    }
}
