#[path = "link_index.rs"]
mod link_index;
#[path = "link_rename.rs"]
mod link_rename;
pub(crate) use link_index::{LinkIndex, Scope};
pub(super) use link_rename::{RENAME_SCHEMA, update_rename_bindings};
use super::{Notebook, Note, Result};
use crate::markdown::{self, AnchorSyntax};
use crate::vault::NoteLinkPaths;
use rusqlite::{Connection, OptionalExtension, params};

pub(super) const IDENTITY_SCHEMA: &str = "
CREATE TABLE note_link_identities (
    note_id INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    token TEXT NOT NULL UNIQUE CHECK(length(token)=32)
);
INSERT INTO note_link_identities(note_id,token)
    SELECT id,lower(hex(randomblob(16))) FROM notes;
CREATE TRIGGER assign_note_link_identity AFTER INSERT ON notes BEGIN
    INSERT INTO note_link_identities(note_id,token) VALUES(NEW.id,lower(hex(randomblob(16))));
END;";

pub(super) const NATIVE_NOTE_PREFIX: &str = "paperroom:note:";

pub(super) fn native_note_target(db: &Connection, path: &str) -> Result<Option<i64>> {
    let Some(dest) = path.strip_prefix(NATIVE_NOTE_PREFIX) else { return Ok(None); };
    let (number, token) = dest.split_once(':').map_or((dest, None), |(id, token)| (id, Some(token)));
    let Ok(id) = number.parse::<i64>() else { return Ok(None); };
    if let Some(token) = token {
        if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Ok(None);
        }
        let matches = db.prepare_cached(
            "SELECT 1 FROM note_link_identities WHERE note_id=?1 AND token=?2",
        )?.exists(params![id, token])?;
        if !matches { return Ok(None); }
    }
    Ok(Some(id))
}

pub struct VaultNoteLink {
    pub note_id: i64,
    pub label: String,
    pub anchor: Option<String>,
    pub anchor_syntax: AnchorSyntax,
}

fn mapped_link_target(
    index: &NoteLinkPaths<'_>, source: &str, parts: &[String], syntax: AnchorSyntax, destination: &str, path: &str,
) -> Option<i64> {
    match syntax {
        AnchorSyntax::Wiki => index.wiki.resolve(source, destination),
        AnchorSyntax::Markdown => index.markdown_target(parts, path),
    }
}

pub(crate) fn note_link_markup(id: i64, token: &str, title: &str) -> String {
    let label: String = title.chars().map(|ch| {
        if ch.is_control() || matches!(ch, '[' | ']' | '|' | '\\') { ' ' } else { ch }
    }).collect();
    let label = if label.trim().is_empty() { "筆記" } else { label.trim() };
    format!("[[{NATIVE_NOTE_PREFIX}{id}:{token}|{label}]]")
}

impl Notebook {
    pub(crate) fn graph_vault(&self, id: i64) -> Result<Option<i64>> {
        Ok(self.connection.query_row("SELECT import_root FROM vault_paths
            WHERE note_id=?1 OR import_root=?1 ORDER BY note_id=?1 DESC LIMIT 1",
            [id], |r|r.get(0)).optional()?)
    }

    pub fn vault_note_links(&self, note: &Note) -> Result<Vec<VaultNoteLink>> {
        let tx = self.connection.unchecked_transaction()?;
        let source = tx.query_row(
            "SELECT p.import_root,p.relative_path,n.body FROM notes n
             LEFT JOIN vault_paths p ON p.note_id=n.id
             LEFT JOIN notes root ON root.id=p.import_root
             WHERE n.id=?1 AND n.trash_root IS NULL AND root.trash_root IS NULL",
            [note.row.id],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?)),
        ).optional()?;
        let Some((root, source_path, body)) = source else { return Ok(Vec::new()); };
        let references = markdown::note_references(&body);
        let mut files = Vec::new();
        if !references.is_empty() && let Some(root) = root {
            let mut stmt = tx.prepare(
                "SELECT p.note_id,p.relative_path FROM vault_paths p
                 JOIN notes n ON n.id=p.note_id
                 WHERE p.import_root=?1",
            )?;
            files = stmt.query_map([root], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        let index = NoteLinkPaths::new(&files);
        let parts = source_path.as_deref().unwrap_or("").split('/').map(str::to_owned).collect::<Vec<_>>();
        let mut active = tx.prepare("SELECT id FROM notes WHERE id=?1 AND trash_root IS NULL")?;
        let mut found = Vec::new();
        for reference in references {
            let destination = reference.destination;
            let fragment = destination.find('#');
            let anchor = fragment.map(|index| destination[index..].to_owned());
            let path = fragment.map_or(destination.as_str(), |index| &destination[..index]);
            let id = if path.is_empty() && anchor.is_some() {
                Some(note.row.id)
            } else if path.starts_with(NATIVE_NOTE_PREFIX) {
                native_note_target(&tx, path)?
            } else if let Some(source_path) = &source_path {
                mapped_link_target(&index, source_path, &parts, reference.syntax, &destination, path)
            } else {
                None
            };
            if let Some(note_id) = id && active.exists([note_id])? {
                let label = if reference.label.trim().is_empty() { destination }
                            else { reference.label };
                found.push(VaultNoteLink { note_id, label, anchor, anchor_syntax: reference.syntax });
            }
        }
        drop(active);
        tx.commit()?;
        Ok(found)
    }
}

#[path = "vault_block_audit.rs"]
mod vault_block_audit;
#[path = "vault_mentions.rs"]
mod vault_mentions;
pub(crate) use vault_mentions::VaultMention;

// A local graph is a projection of the same links used by the note menu.
// Keep a stable target ID even when several paths have the same title.
type Neighbor = (i64, String, String);

impl Notebook {
    pub fn graph_neighbors(&self, note: &Note) -> Result<(Vec<Neighbor>, Vec<Neighbor>)> {
        let index = self.link_index(Scope::Workspace, Some(note))?;
        let describe = |id| {
            let node = &index.nodes[&id];
            (id, node.row.title.clone(), node.detail())
        };
        let mut outgoing=Vec::new();
        let mut incoming=Vec::new();
        for &(source,target) in &index.edges {
            if source==note.row.id { outgoing.push(describe(target)); }
            if target==note.row.id && source!=target { incoming.push(describe(source)); }
        }
        Ok((outgoing, incoming))
    }
}

#[cfg(test)]
mod graph_tests {
    use super::*;
    use crate::archive::ImportedPage;


    #[test]
    fn heading_links_keep_vault_identity_and_source_caret_after_offline_reopen() -> Result<()> {
        use crate::markdown::{HeadingLookup, resolve_heading};
        use std::fs;

        let dir = super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let db_path = dir.join("work.sqlite");
        let target = "乙/鏡頭.md";
        let text = "\u{feff}---\r\naliases: [目標]\r\n---\r\n🎥 序\r\n# 第一幕\r\n## 同名\r\n錯誤\r\n# 第二幕\r\n## 同名\r\n正確\r\n";
        let body = concat!(
            "# 入口\r\n",
            "[[乙/鏡頭#第二幕#同名|前往指定段]] [正常格式](乙/鏡頭.md#第二幕#同名) ",
            "[[#入口|同篇]] [同篇 Markdown](#入口) [[鏡頭#第二幕#同名|模稜兩可]] ",
            "[外部](https://example.invalid/#入口) `[[#入口|程式碼]]`",
        );
        for folder in ["甲", "乙"] {
            fs::create_dir_all(vault.join(folder))?;
        }
        fs::write(vault.join(target), text)?;
        fs::write(vault.join("甲/鏡頭.md"), "# 第二幕\n## 同名\n另一份筆記")?;
        fs::write(vault.join("入口.md"), body)?;
        let mut db = Notebook::open(&db_path, false)?;
        let root = db.import_vault("合成 Vault", crate::vault::read(&vault)?.pages)?;
        let id = |path: &str| db.connection.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            (root, path), |row| row.get::<_, i64>(0),
        );
        let dest = id(target)?;
        let entry = id("入口.md")?;
        let plain = db.add(None, "純本機筆記", "# 本機\n[[#本機|仍是同一篇]]")?;
        drop(db);

        let fresh = Notebook::open(&db_path, false)?;
        let source = fresh.note(entry)?.unwrap();
        let refs = fresh.vault_note_links(&source)?;
        assert_eq!(refs.len(), 4);
        assert_eq!((refs[0].label.as_str(), refs[0].anchor_syntax), ("前往指定段", AnchorSyntax::Wiki));
        assert_eq!((refs[1].label.as_str(), refs[1].anchor_syntax), ("正常格式", AnchorSyntax::Markdown));
        let saved = fresh.note(dest)?.unwrap();
        let headings = super::super::navigation::outline(&saved.body);
        let editor_utf16 = text.encode_utf16().collect::<Vec<_>>();
        for r in &refs[..2] {
            assert_eq!(r.note_id, dest);
            let HeadingLookup::Found { ordinal, section } =
                resolve_heading(&saved.body, r.anchor.as_deref().unwrap(), r.anchor_syntax)
                else { panic!("target section not resolved"); };
            let title = &headings[ordinal];
            assert_eq!((title.line, title.title.as_str()), (9, "同名"));
            assert_eq!(String::from_utf16(&editor_utf16[title.start..title.end]).unwrap(), "## 同名");
            let excerpt = &saved.body[section];
            assert!(excerpt.contains("正確") && !excerpt.contains("錯誤"));
        }
        assert!(refs[2..].iter().all(|r| r.note_id == entry && r.anchor.as_deref() == Some("#入口")));
        let plain = fresh.note(plain)?.unwrap();
        assert_eq!(fresh.vault_note_links(&plain)?[0].note_id, plain.row.id);
        assert_eq!(source.body, body);
        assert_eq!(fs::read(vault.join(target))?, text.as_bytes());
        assert_eq!(fs::read(vault.join("入口.md"))?, body.as_bytes());
        drop(fresh);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn explicit_vault_block_links_preserve_source_id_crlf_selection_and_original_files() -> Result<()> {
        use crate::markdown::block_anchors::{BlockKind, BlockLookup, resolve_block};
        use super::super::navigation::editor_source_selection;

        let dir = super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let detached = dir.join("離線");
        let source = "乙/鏡頭.md";
        let target_body = "# 場景\r\n\r\n🎬 第一行\r\n要選取的內容 ^shot-a\r\n\r\n另一段";
        let entry_body = "# 入口\r\n\r\n[[乙/鏡頭#^shot-a|去鏡頭段落]] [[#^local-block|同篇區塊]] [[鏡頭#^shot-a|歧義]]\r\n\r\n來源頁段落 ^local-block";
        let sqlite = dir.join("工作區.sqlite");
        let backup = dir.join("備份.sqlite");
        for folder in ["甲", "乙"] {
            std::fs::create_dir_all(vault.join(folder))?;
        }
        std::fs::write(vault.join(source), target_body)?;
        std::fs::write(vault.join("甲/鏡頭.md"), "另一篇同名筆記 ^shot-a")?;
        std::fs::write(vault.join("入口.md"), entry_body)?;
        let mut db = Notebook::open(&sqlite, false)?;
        let root = db.import_vault("合成 Vault", crate::vault::read(&vault)?.pages)?;
        let exact = |path: &str| -> Result<i64> {
            let hits = db.search_vault_sources(root, "", "", "", crate::store::vault_search::TagMatch::Exact)?;
            hits.into_iter().find(|(_, relative)| relative == path).map(|(id, _)| id).ok_or("找不到來源".into())
        };
        let target = exact(source)?;
        let entry = exact("入口.md")?;
        db.backup(&backup)?;
        drop(db);
        std::fs::rename(&vault, &detached)?;

        for sqlite in [sqlite, backup] {
            let db = Notebook::open(&sqlite, false)?;
            let from = db.note(entry)?.unwrap();
            let links = db.vault_note_links(&from)?;
            assert_eq!(links.len(), 2);
            assert_eq!((links[0].note_id, links[0].anchor.as_deref()), (target, Some("#^shot-a")));
            assert_eq!((links[1].note_id, links[1].anchor.as_deref()), (entry, Some("#^local-block")));
            for (link, line, expected) in [
                (&links[0], 3, "🎬 第一行\r\n要選取的內容"),
                (&links[1], 5, "來源頁段落"),
            ] {
                let body = db.note(link.note_id)?.unwrap().body;
                let BlockLookup::Found { source, kind: BlockKind::Paragraph } =
                    resolve_block(&body, link.anchor.as_deref().unwrap())
                    else { panic!("找不到已保存區塊"); };
                let cursor = editor_source_selection(&body, source.clone()).unwrap();
                let raw_utf16 = body.encode_utf16().collect::<Vec<_>>();
                assert_eq!((&body[source], cursor.line), (expected, line));
                assert_eq!(String::from_utf16(&raw_utf16[cursor.start..cursor.end]).unwrap(), expected);
            }
        }
        assert_eq!(std::fs::read(detached.join(source))?, target_body.as_bytes());
        assert_eq!(std::fs::read(detached.join("入口.md"))?, entry_body.as_bytes());
        assert!(!vault.exists());
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn local_graph_keeps_vault_scope_and_direction_after_reopen() -> Result<()> {
        let path = std::env::temp_dir().join(format!("paperroom-graph-{}-{}.sqlite3",
            std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        let mut db = Notebook::open(&path, false)?;
        let pages = [("A", "主頁", "[[../B/同名#第二節|別名]] [[同名]]"),
                     ("A", "同名", ""), ("B", "同名", "[[../A/主頁]]")].into_iter()
            .map(|(dir, name, body)| ImportedPage {
                parts: vec![dir.into(), format!("{name}.md")], title: name.into(), body: body.into(),
            }).collect();
        db.import_vault("來源", pages)?;
        let id = db.rows(super::super::View::All, "主頁")?.into_iter()
            .find(|r|r.title == "主頁").unwrap().id;
        let source = db.note(id)?.unwrap();
        let references = db.vault_note_links(&source)?;
        let target = db.note(references[0].note_id)?.unwrap();
        db.save(target.row.id, target.version, "重新命名", &target.body)?;
        drop(db);
        let reopened = Notebook::open(&path, false)?;
        let (outgoing,incoming) = reopened.graph_neighbors(&reopened.note(id)?.unwrap())?;
        assert_eq!(outgoing, incoming);
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].1, "重新命名");
        assert!(outgoing[0].2.contains("B/同名.md"));
        let persisted = reopened.note(id)?.unwrap();
        assert_eq!(persisted.body, source.body);
        let references = reopened.vault_note_links(&persisted)?;
        assert_eq!((references[0].note_id, references[0].label.as_str(), references[0].anchor.as_deref()),
                   (target.row.id, "別名", Some("#第二節")));
        drop(reopened);
        std::fs::remove_file(path)?;
        Ok(())
    }
}
