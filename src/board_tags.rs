//! Native board-to-tag links; note tagging remains owned by its existing source.
use super::{Notebook, Result};
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
