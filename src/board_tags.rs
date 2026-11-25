//! Native board-to-tag links; note tagging remains owned by its existing source.
use super::{Notebook, Result, vault_search::TagMatch};
use rusqlite::{Connection, params};

pub(super) const SCHEMA: &str = "
CREATE TABLE board_tags(
 board_id INTEGER NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
 tag_key TEXT NOT NULL,
 label TEXT NOT NULL,
 PRIMARY KEY(board_id,tag_key)
);
CREATE INDEX board_tags_by_key ON board_tags(tag_key,board_id);";
const MAX_TAGS_PER_BOARD: i64 = 128;

/// Mirrors existing Vault exact tag semantics: slash paths, Unicode lowercase,
/// no ASCII-only SQL collation. Keep the display label and identity separate.
fn normalize(raw: &str) -> Result<(&str, String)> {
    let raw = raw.trim();
    let tag = raw.strip_prefix('#').unwrap_or(raw);
    if tag.is_empty() || tag.len() > 4096
        || !tag.chars().all(|ch| ch.is_alphanumeric() || matches!(ch,'_'|'-'|'/'))
        || !tag.chars().any(|ch| !ch.is_numeric())
        || tag.split('/').any(str::is_empty) {
        return Err("標籤名稱無效，例如 #拍攝/夜景。".into());
    }
    Ok((tag, tag.to_lowercase()))
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TagSubject { Board(i64), Note(i64) }

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TypedTagHit { pub subject: TagSubject, pub title: String }

fn set_board_tag_on(db: &Connection, board:i64, raw:&str, enabled:bool) -> Result<()> {
    let (label,key) = normalize(raw)?;
    if !enabled {
        db.execute("DELETE FROM board_tags WHERE board_id=?1 AND tag_key=?2",params![board,key])?;
        return Ok(());
    }
    let changed = db.execute(
        "INSERT INTO board_tags(board_id,tag_key,label)
         SELECT ?1,?2,?3
         WHERE EXISTS(SELECT 1 FROM boards WHERE id=?1)
           AND (SELECT COUNT(*) FROM board_tags WHERE board_id=?1) < ?4
         ON CONFLICT(board_id,tag_key) DO NOTHING",
        params![board,key,label,MAX_TAGS_PER_BOARD],
    )?;
    if changed == 0 && !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM board_tags WHERE board_id=?1 AND tag_key=?2)",
        params![board,key], |r| r.get::<_,bool>(0),
    )? {
        return Err("白板不存在或已有太多標籤。".into());
    }
    Ok(())
}

/// Call with authorized note matches computed by the existing note search.
fn typed_hits(db:&Connection, raw:&str, notes:&[(i64,String)]) -> Result<Vec<TypedTagHit>> {
    let (_,key) = normalize(raw)?;
    let mut stmt = db.prepare(
        "SELECT b.id,b.title FROM board_tags t
         JOIN boards b ON b.id=t.board_id WHERE t.tag_key=?1 ORDER BY b.title,b.id"
    )?;
    let mut hits = stmt.query_map([key],|r| Ok(TypedTagHit {
        subject:TagSubject::Board(r.get(0)?),title:r.get(1)?,
    }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    hits.extend(notes.iter().map(|(id,title)| TypedTagHit {
        subject:TagSubject::Note(*id),title:title.clone(),
    }));
    Ok(hits)
}

impl Notebook {
    pub(crate) fn set_board_tag(&self, board:i64, tag:&str, enabled:bool)->Result<()> {
        set_board_tag_on(&self.connection,board,tag,enabled)
    }
    pub(crate) fn board_tags(&self,board:i64)->Result<Vec<String>> {
        let mut stmt = self.connection.prepare(
            "SELECT label FROM board_tags WHERE board_id=?1 ORDER BY tag_key"
        )?;
        Ok(stmt.query_map([board],|r|r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Boards are global; note hits are permitted only from the explicitly selected Vault.
    pub(crate) fn search_board_and_vault_tag(
        &self, tag: &str, vault: Option<i64>,
    ) -> Result<Vec<TypedTagHit>> {
        let notes = match vault {
            Some(root) => self.search_vault_tag_terms(root, &[(tag.to_owned(), TagMatch::Exact)])?,
            None => Vec::new(),
        };
        typed_hits(&self.connection, tag, &notes)
    }

    pub(crate) fn exact_board_and_note_tag_hits(
        &self, tag:&str, notes:&[(i64,String)],
    )->Result<Vec<TypedTagHit>> {
        typed_hits(&self.connection,tag,notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_tag_lookup_is_exact_and_uses_only_the_selected_imported_vault() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let one = dir.join("one");
        let two = dir.join("two");
        std::fs::create_dir(&one)?;
        std::fs::create_dir(&two)?;
        std::fs::write(one.join("同名.md"), "#拍攝/夜景")?;
        std::fs::write(one.join("近似.md"), "#拍攝/夜景/待剪")?;
        std::fs::write(two.join("同名.md"), "#拍攝/夜景")?;
        let path = dir.join("workspace.sqlite3");

        let (board, sibling, vault) = {
            let mut db = Notebook::open(&path, false)?;
            let board = db.create_board("同名")?;
            let sibling = db.create_board("同名（副）")?;
            let near = db.create_board("近似")?;
            db.set_board_tag(board, "#拍攝/夜景", true)?;
            db.set_board_tag(sibling, "#拍攝/夜景", true)?;
            db.set_board_tag(near, "#拍攝/夜景/待剪", true)?;
            db.add(None, "本機同名", "#拍攝/夜景")?;
            let vault = db.import_vault("首批", crate::vault::read(&one)?.pages)?;
            db.import_vault("另一批", crate::vault::read(&two)?.pages)?;
            (board, sibling, vault)
        };
        {
            let db = Notebook::open(&path, false)?;
            let tag = "#拍攝/夜景";
            let board_hits = db.search_board_and_vault_tag(tag, None)?;
            assert_eq!(board_hits.len(), 2);
            assert!(board_hits.iter().any(|h| h.subject == TagSubject::Board(board)));
            assert!(board_hits.iter().any(|h| h.subject == TagSubject::Board(sibling)));
            let mixed = db.search_board_and_vault_tag(tag, Some(vault))?;
            assert_eq!(mixed.len(), 3);
            let hit = mixed.last().ok_or("找不到精確標籤筆記")?;
            let TagSubject::Note(note) = hit.subject else { panic!("筆記和白板必須分開"); };
            assert_eq!(hit.title, "同名.md");
            assert!(db.vault_search_hit(vault, note, &hit.title)?);

            db.set_board_tag(board, tag, false)?;
            let after = db.search_board_and_vault_tag(tag, Some(vault))?;
            assert_eq!(after.iter().map(|hit| hit.subject).collect::<Vec<_>>(),
                [TagSubject::Board(sibling), TagSubject::Note(note)]);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn tags_use_stable_board_identity_even_when_note_title_and_id_match() {
        let db=Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE boards(id INTEGER PRIMARY KEY,title TEXT NOT NULL UNIQUE);
            INSERT INTO boards VALUES(1,'研究板'),(2,'研究板（副）');").unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let notes=vec![(1,"研究板".into())];
        assert!(set_board_tag_on(&db,999,"觀察",true).is_err());
        set_board_tag_on(&db,1," #觀察/AbC ",true).unwrap();
        set_board_tag_on(&db,1,"觀察/abc",true).unwrap();
        assert_eq!(typed_hits(&db,"觀察/ABC",&notes).unwrap().iter().map(|h|h.subject)
            .collect::<Vec<_>>(), vec![TagSubject::Board(1),TagSubject::Note(1)]);
        db.execute("UPDATE boards SET title='改名後研究板' WHERE id=1",[]).unwrap();
        assert_eq!(typed_hits(&db,"觀察/abc",&notes).unwrap()[0].title,"改名後研究板");
        set_board_tag_on(&db,1,"觀察/abc",false).unwrap();
        assert_eq!(typed_hits(&db,"觀察/abc",&notes).unwrap()[0].subject,TagSubject::Note(1));
        set_board_tag_on(&db,1,"觀察/abc",true).unwrap();
        db.execute("DELETE FROM boards WHERE id=1",[]).unwrap();
        assert!(typed_hits(&db,"觀察/abc",&[]).unwrap().is_empty());
    }
}
