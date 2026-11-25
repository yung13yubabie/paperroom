use super::attachments::PdfAnnotationHit;
use super::{Notebook, Result, boards::place_card_on, insert};
#[cfg(any(windows, test))]
use super::pdf_regions::PdfPageText;
use super::vault_search::{MAX_SCAN, TagFilter, TagMatch, matches_tag};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::HashSet;

const EXCERPT_LIBRARY_MAX: usize = 128;
const LIBRARY_SOURCES: &str = "
    SELECT n.id AS card_id,n.title AS card_title,n.body AS search_body,n.pinned AS pinned,
           c.note_id IS NOT NULL AS is_pdf,
           CASE WHEN length(n.body)>4096
                THEN substr(n.body,1,4096)||char(10)||'…（預覽節錄）' ELSE n.body END AS body,
           CASE WHEN source.id IS NOT NULL AND (pair.note_id IS NULL OR
                (second.attachment_id=a.id AND second.page=first.page+1)) THEN a.id END AS attachment,
           source.id AS source_note,a.name AS source_file,source.title AS source_title,
           coalesce(first.page,live.page) AS page,second.page AS page_next,
           coalesce(first.quote,live.quote,'') AS quote,coalesce(first.comment,live.comment,'') AS comment,
           coalesce(second.quote,'') AS quote_next,coalesce(second.comment,'') AS comment_next,
           c.annotation_id IS NOT NULL AS mark_live, second.live_annotation IS NOT NULL AS mark_next_live
    FROM notes n
    LEFT JOIN pdf_excerpt_cards c ON c.note_id=n.id
    LEFT JOIN pdf_annotations live ON live.id=c.annotation_id
    LEFT JOIN pdf_excerpt_origins first ON first.id=c.origin_id
    LEFT JOIN pdf_excerpt_second_page pair ON pair.note_id=c.note_id
    LEFT JOIN pdf_excerpt_origins second ON second.id=pair.origin_id
    LEFT JOIN attachments a ON a.id=coalesce(first.attachment_id,live.attachment_id) AND a.kind='pdf'
    LEFT JOIN notes source ON source.id=a.note_id AND source.trash_root IS NULL
    WHERE n.trash_root IS NULL";

pub(crate) struct PdfExcerptLibrarySource {
    pub attachment: i64,
    pub note: i64,
    pub file: String,
    pub title: String,
}

pub(crate) struct PdfExcerptLibraryFilter<'a> {
    pub attachment: Option<i64>,
    pub unavailable_only: bool,
    pub on_board: Option<bool>,
    pub page: Option<u32>,
    pub text: &'a str,
}

pub(crate) struct BoardCardLibraryFilter<'a> {
    pub(crate) pdf: PdfExcerptLibraryFilter<'a>,
    pub(crate) pdf_only: Option<bool>,
    pub(crate) starred: Option<bool>,
    pub(crate) tag: &'a str,
}

pub(crate) struct PdfExcerptLibraryHit {
    pub card: i64,
    pub title: String,
    pub source: Option<PdfExcerptLibrarySource>,
    pub on_board: bool,
    pub pages: [Option<u32>; 2],
    pub quotes: [String; 2],
    pub comments: [String; 2],
    pub live: [bool; 2],
    pub body: String,
    pub is_pdf: bool,
    pub starred: bool,
}

fn checked_library_ids(notes: &[i64]) -> Result<()> {
    if notes.is_empty() || notes.len() > EXCERPT_LIBRARY_MAX ||
        notes.iter().copied().collect::<HashSet<_>>().len() != notes.len()
    {
        return Err("請選取不重複的卡片，最多 128 張。".into());
    }
    Ok(())
}

pub(super) const SCHEMA: &str = "
CREATE TABLE pdf_excerpt_cards(
    note_id INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    annotation_id INTEGER REFERENCES pdf_annotations(id) ON DELETE SET NULL
);
CREATE INDEX pdf_excerpt_cards_annotation
    ON pdf_excerpt_cards(annotation_id) WHERE annotation_id IS NOT NULL;
";

pub(super) const RECOVERY_SCHEMA: &str = "
CREATE TABLE pdf_excerpt_origins(
    id INTEGER PRIMARY KEY,
    live_annotation INTEGER UNIQUE REFERENCES pdf_annotations(id) ON DELETE SET NULL,
    attachment_id INTEGER REFERENCES attachments(id) ON DELETE SET NULL,
    page INTEGER NOT NULL,
    quote TEXT NOT NULL,
    comment TEXT NOT NULL,
    is_text INTEGER NOT NULL CHECK(is_text IN (0,1))
);
CREATE TABLE pdf_excerpt_origin_regions(
    origin_id INTEGER NOT NULL REFERENCES pdf_excerpt_origins(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal>=-1 AND ordinal<64),
    x REAL NOT NULL CHECK(x>=0 AND x<1),
    y REAL NOT NULL CHECK(y>=0 AND y<1),
    width REAL NOT NULL CHECK(width>0 AND x+width<=1),
    height REAL NOT NULL CHECK(height>0 AND y+height<=1),
    PRIMARY KEY(origin_id,ordinal)
) WITHOUT ROWID;
ALTER TABLE pdf_excerpt_cards ADD COLUMN origin_id INTEGER REFERENCES pdf_excerpt_origins(id);
CREATE INDEX pdf_excerpt_cards_origin ON pdf_excerpt_cards(origin_id);
CREATE TRIGGER delete_unreferenced_pdf_excerpt_origin AFTER DELETE ON pdf_excerpt_cards
WHEN OLD.origin_id IS NOT NULL
BEGIN
    DELETE FROM pdf_excerpt_origins WHERE id=OLD.origin_id
    AND NOT EXISTS(SELECT 1 FROM pdf_excerpt_cards WHERE origin_id=OLD.origin_id);
END;
";

pub(super) const SECOND_PAGE_SCHEMA: &str = "
CREATE TABLE pdf_excerpt_second_page(
    note_id INTEGER PRIMARY KEY REFERENCES pdf_excerpt_cards(note_id) ON DELETE CASCADE,
    origin_id INTEGER NOT NULL REFERENCES pdf_excerpt_origins(id)
);
CREATE INDEX pdf_excerpt_second_page_origin ON pdf_excerpt_second_page(origin_id);
DROP TRIGGER delete_unreferenced_pdf_excerpt_origin;
CREATE TRIGGER delete_unreferenced_pdf_excerpt_origin AFTER DELETE ON pdf_excerpt_cards
WHEN OLD.origin_id IS NOT NULL
BEGIN
    DELETE FROM pdf_excerpt_origins WHERE id=OLD.origin_id
    AND NOT EXISTS(SELECT 1 FROM pdf_excerpt_cards WHERE origin_id=OLD.origin_id)
    AND NOT EXISTS(SELECT 1 FROM pdf_excerpt_second_page WHERE origin_id=OLD.origin_id);
END;
CREATE TRIGGER delete_unreferenced_pdf_excerpt_second_origin AFTER DELETE ON pdf_excerpt_second_page
BEGIN
    DELETE FROM pdf_excerpt_origins WHERE id=OLD.origin_id
    AND NOT EXISTS(SELECT 1 FROM pdf_excerpt_cards WHERE origin_id=OLD.origin_id)
    AND NOT EXISTS(SELECT 1 FROM pdf_excerpt_second_page WHERE origin_id=OLD.origin_id);
END;
";

#[cfg(test)]
pub(super) const UNDO_SECOND_PAGE_SCHEMA: &str = "
DROP TRIGGER delete_unreferenced_pdf_excerpt_origin;
DROP TABLE pdf_excerpt_second_page;
CREATE TRIGGER delete_unreferenced_pdf_excerpt_origin AFTER DELETE ON pdf_excerpt_cards
WHEN OLD.origin_id IS NOT NULL
BEGIN
    DELETE FROM pdf_excerpt_origins WHERE id=OLD.origin_id
    AND NOT EXISTS(SELECT 1 FROM pdf_excerpt_cards WHERE origin_id=OLD.origin_id);
END;
";

pub(super) fn backfill_pdf_excerpt_sources(conn: &Connection) -> Result<()> {
    let mut select = conn.prepare(
        "SELECT DISTINCT annotation_id FROM pdf_excerpt_cards
         WHERE annotation_id IS NOT NULL AND origin_id IS NULL",
    )?;
    let rows = select.query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for annotation in rows {
        let origin = snapshot_pdf_source(conn, annotation)?;
        conn.execute(
            "UPDATE pdf_excerpt_cards SET origin_id=?2 WHERE annotation_id=?1 AND origin_id IS NULL",
            params![annotation, origin],
        )?;
    }
    Ok(())
}

fn snapshot_pdf_source(conn: &Connection, annotation: i64) -> Result<i64> {
    if let Some(origin) = conn.query_row(
        "SELECT id FROM pdf_excerpt_origins WHERE live_annotation=?1",
        [annotation], |row| row.get(0),
    ).optional()? {
        return Ok(origin);
    }
    let inserted = conn.execute(
        "INSERT INTO pdf_excerpt_origins(live_annotation,attachment_id,page,quote,comment,is_text)
         SELECT p.id,p.attachment_id,p.page,p.quote,p.comment,
                EXISTS(SELECT 1 FROM pdf_text_marks t WHERE t.annotation_id=p.id)
         FROM pdf_annotations p WHERE p.id=?1",
        [annotation],
    )?;
    if inserted == 0 {
        return Err("找不到摘錄的原始 PDF 標註。".into());
    }
    let origin = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO pdf_excerpt_origin_regions(origin_id,ordinal,x,y,width,height)
         SELECT ?2,-1,x,y,width,height FROM pdf_annotation_regions WHERE annotation_id=?1
         UNION ALL
         SELECT ?2,ordinal,x,y,width,height FROM pdf_text_mark_segments WHERE annotation_id=?1",
        params![annotation, origin],
    )?;
    Ok(origin)
}

#[cfg(any(windows, test))]
pub struct PdfExcerptRecovery {
    pub origin: i64,
    pub name: String,
    pub page: u32,
    pub quote: String,
    pub comment: String,
    attachment: i64,
    source_note: i64,
    is_text: bool,
}

#[cfg(any(windows, test))]
pub struct PdfExcerptSnapshot {
    pub note: i64,
    pub attachment: i64,
    pub page: u32,
    pub focus: Vec<super::PdfRegion>,
}

#[cfg(any(windows, test))]
pub(crate) struct PdfAdjacentExcerptPage {
    pub attachment: i64,
    pub source_note: i64,
    pub page: u32,
    pub annotation: Option<i64>,
    pub focus: Vec<super::PdfRegion>,
}

#[cfg(any(windows, test))]
fn excerpt_origin_regions(conn: &Connection, origin: i64) -> Result<Vec<super::PdfRegion>> {
    let mut select = conn.prepare(
        "SELECT x,y,width,height FROM pdf_excerpt_origin_regions
         WHERE origin_id=?1 AND (ordinal>=0 OR NOT EXISTS(
             SELECT 1 FROM pdf_excerpt_origin_regions WHERE origin_id=?1 AND ordinal>=0
         )) ORDER BY ordinal",
    )?;
    Ok(select.query_map([origin], |row| Ok(super::PdfRegion {
        x: row.get(0)?, y: row.get(1)?, width: row.get(2)?, height: row.get(3)?,
    }))?.collect::<rusqlite::Result<_>>()?)
}

#[cfg(any(windows, test))]
fn recovery_preview(conn: &Connection, note: i64) -> Result<Option<PdfExcerptRecovery>> {
    Ok(conn.query_row(
        "SELECT o.id,a.name,o.page,o.quote,o.comment,a.id,o.is_text,source.id
         FROM pdf_excerpt_cards c
         JOIN notes card ON card.id=c.note_id AND card.trash_root IS NULL
         JOIN pdf_excerpt_origins o ON o.id=c.origin_id AND o.live_annotation IS NULL
         JOIN attachments a ON a.id=o.attachment_id AND a.kind='pdf'
         JOIN notes source ON source.id=a.note_id AND source.trash_root IS NULL
         WHERE c.note_id=?1 AND c.annotation_id IS NULL
           AND NOT EXISTS(SELECT 1 FROM pdf_excerpt_second_page p WHERE p.note_id=c.note_id)",
        [note], |row| Ok(PdfExcerptRecovery {
            origin: row.get(0)?, name: row.get(1)?, page: row.get(2)?, quote: row.get(3)?,
            comment: row.get(4)?, attachment: row.get(5)?, is_text: row.get(6)?, source_note: row.get(7)?,
        }),
    ).optional()?)
}

impl Notebook {
    pub(crate) fn pdf_excerpt_library_sources(&self) -> Result<Vec<PdfExcerptLibrarySource>> {
        let sql = format!("WITH library AS ({LIBRARY_SOURCES})
            SELECT DISTINCT attachment,source_note,source_file,source_title
            FROM library WHERE attachment IS NOT NULL ORDER BY source_title,source_file,attachment");
        let mut select = self.connection.prepare(&sql)?;
        let rows = select.query_map([], |row| Ok(PdfExcerptLibrarySource {
            attachment: row.get(0)?, note: row.get(1)?, file: row.get(2)?, title: row.get(3)?,
        }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    #[cfg(test)]
    pub(crate) fn pdf_excerpt_library(
        &self, board: i64, filter: PdfExcerptLibraryFilter<'_>,
    ) -> Result<(Vec<PdfExcerptLibraryHit>, bool)> {
        self.board_card_library(board, BoardCardLibraryFilter {
            pdf: filter, pdf_only: Some(true), starred: None, tag: "",
        })
    }

    pub(crate) fn board_card_library(
        &self, board: i64, filter: BoardCardLibraryFilter<'_>,
    ) -> Result<(Vec<PdfExcerptLibraryHit>, bool)> {
        let BoardCardLibraryFilter { pdf: filter, pdf_only, starred, tag } = filter;
        let text = filter.text.trim();
        if text.chars().count() > 180 || text.contains('\0') || filter.page == Some(0) {
            return Err("請輸入 180 字以內的搜尋文字，以及正整數頁碼。".into());
        }
        let tag = if tag.trim().is_empty() { None } else { Some(TagFilter::parse(tag, TagMatch::Exact)?) };
        let sql = format!("WITH library AS ({LIBRARY_SOURCES}), board_scope AS (
            SELECT library.*,EXISTS(SELECT 1 FROM board_cards b WHERE b.board_id=?1 AND b.note_id=library.card_id) AS on_board
            FROM library
        ) SELECT card_id,card_title,body,attachment,source_note,source_file,source_title,
                 page,page_next,quote,quote_next,comment,comment_next,mark_live,mark_next_live,on_board,is_pdf,pinned,
                 CASE WHEN ?10 THEN search_body END
          FROM board_scope WHERE (?2 IS NULL OR attachment=?2)
            AND (NOT ?3 OR (is_pdf AND attachment IS NULL)) AND (?4 IS NULL OR on_board=?4)
            AND (?5 IS NULL OR page=?5 OR page_next=?5)
            AND (instr(lower(card_title),lower(?6))>0 OR instr(lower(search_body),lower(?6))>0
                 OR instr(lower(quote),lower(?6))>0 OR instr(lower(quote_next),lower(?6))>0
                 OR instr(lower(comment),lower(?6))>0 OR instr(lower(comment_next),lower(?6))>0)
            AND (?7 IS NULL OR is_pdf=?7) AND (?8 IS NULL OR pinned=?8)
          ORDER BY card_id DESC LIMIT ?9");
        let mut select = self.connection.prepare(&sql)?;
        let limit = if tag.is_some() { -1 } else { EXCERPT_LIBRARY_MAX as i64 + 1 };
        let mut rows = select.query(
            params![board, filter.attachment, filter.unavailable_only, filter.on_board, filter.page, text,
                pdf_only, starred, limit, tag.is_some()],
        )?;
        let mut hits = Vec::new();
        let mut scanned = 0usize;
        while let Some(row) = rows.next()? {
            if let Some(tag) = &tag {
                let body = row.get_ref(18)?.as_str()?;
                scanned = scanned.saturating_add(body.len());
                if scanned > MAX_SCAN {
                    return Ok((hits, true));
                }
                if !matches_tag(body, tag)? {
                    continue;
                }
            }
            if hits.len() == EXCERPT_LIBRARY_MAX {
                return Ok((hits, true));
            }
            let source = if let Some(attachment) = row.get(3)? {
                Some(PdfExcerptLibrarySource {
                    attachment, note: row.get(4)?, file: row.get(5)?, title: row.get(6)?,
                })
            } else { None };
            hits.push(PdfExcerptLibraryHit {
                card: row.get(0)?, title: row.get(1)?, body: row.get(2)?, source,
                pages: [row.get(7)?, row.get(8)?], quotes: [row.get(9)?, row.get(10)?],
                comments: [row.get(11)?, row.get(12)?], live: [row.get(13)?, row.get(14)?],
                on_board: row.get(15)?, is_pdf: row.get(16)?, starred: row.get(17)?,
            });
        }
        Ok((hits, false))
    }

    #[cfg(any(windows, test))]
    pub fn search_pdf_excerpt_cards(&self, query: &str) -> Result<Vec<(i64, String)>> {
        let mut select = self.connection.prepare(
            "SELECT n.id,n.title FROM pdf_excerpt_cards c
             JOIN notes n ON n.id=c.note_id AND n.trash_root IS NULL
             LEFT JOIN pdf_excerpt_origins o ON o.id=c.origin_id
             LEFT JOIN pdf_excerpt_second_page p ON p.note_id=c.note_id
             LEFT JOIN pdf_excerpt_origins second ON second.id=p.origin_id
             WHERE instr(lower(n.title),lower(?1))>0
                OR instr(lower(coalesce(o.quote,'')),lower(?1))>0
                OR instr(lower(coalesce(o.comment,'')),lower(?1))>0
                OR instr(lower(coalesce(second.quote,'')),lower(?1))>0
                OR instr(lower(coalesce(second.comment,'')),lower(?1))>0
             ORDER BY n.id DESC LIMIT 64",
        )?;
        let rows = select.query_map([query.trim()], |row| Ok((row.get(0)?,row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Place an existing independent note, even when its source is unavailable.
    /// Returns whether a placement was created.
    #[cfg(any(windows, test))]
    pub fn place_existing_pdf_excerpt(&self, board: i64, note: i64) -> Result<bool> {
        Ok(self.place_existing_pdf_excerpts(board, &[note])? == 1)
    }

    #[cfg(any(windows, test))]
    pub(crate) fn place_existing_pdf_excerpts(&self, board: i64, notes: &[i64]) -> Result<usize> {
        self.place_library_cards(board, notes, true)
    }

    pub(crate) fn place_board_library_cards(&self, board: i64, notes: &[i64]) -> Result<usize> {
        self.place_library_cards(board, notes, false)
    }

    fn place_library_cards(&self, board: i64, notes: &[i64], pdf_only: bool) -> Result<usize> {
        checked_library_ids(notes)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let mut added = 0;
        {
            let mut check = tx.prepare(
                "SELECT EXISTS(SELECT 1 FROM board_cards b WHERE b.board_id=?1 AND b.note_id=n.id)
                 FROM notes n WHERE n.id=?2 AND n.trash_root IS NULL
                      AND (NOT ?3 OR EXISTS(SELECT 1 FROM pdf_excerpt_cards WHERE note_id=n.id))",
            )?;
            for note in notes {
                let already_placed: bool = check.query_row(
                    params![board, note, pdf_only], |row| row.get(0),
                ).optional()?.ok_or("找不到選取的卡片。請重新搜尋，這次沒有放入任何卡片。")?;
                if !already_placed {
                    place_card_on(&tx, board, *note)?;
                    added += 1;
                }
            }
        }
        tx.commit()?;
        Ok(added)
    }

    pub(crate) fn set_board_library_stars(&self, notes: &[i64], starred: bool) -> Result<usize> {
        checked_library_ids(notes)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let mut changed = 0;
        {
            let mut check = tx.prepare("SELECT pinned FROM notes WHERE id=?1 AND trash_root IS NULL")?;
            let mut update = tx.prepare("UPDATE notes SET pinned=?2 WHERE id=?1 AND trash_root IS NULL")?;
            for note in notes {
                let current: bool = check.query_row([note], |row| row.get(0)).optional()?
                    .ok_or("找不到選取的卡片。請重新搜尋，這次沒有變更標星。")?;
                if current != starred {
                    changed += update.execute(params![note, starred])?;
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    #[cfg(any(windows, test))]
    pub fn pdf_excerpt_recovery_preview(&self, note: i64) -> Result<Option<PdfExcerptRecovery>> {
        recovery_preview(&self.connection, note)
    }

    #[cfg(any(windows, test))]
    pub fn pdf_excerpt_snapshot(&self, note: i64) -> Result<Option<PdfExcerptSnapshot>> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        let Some(source) = recovery_preview(&tx, note)? else { return Ok(None) };
        let focus = excerpt_origin_regions(&tx, source.origin)?;
        tx.commit()?;
        Ok(Some(PdfExcerptSnapshot {
            note: source.source_note, attachment: source.attachment, page: source.page, focus,
        }))
    }

    #[cfg(any(windows, test))]
    pub fn restore_pdf_excerpt_annotation(&self, note: i64, origin: i64) -> Result<i64> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let source = recovery_preview(&tx, note)?.filter(|preview| preview.origin == origin)
            .ok_or("標註狀態已變更，或原始 PDF 已無法使用。請重新檢查摘錄。")?;
        let annotation = self.add_pdf_annotation(source.attachment, source.page, &source.quote, &source.comment)?;
        tx.execute(
            "INSERT INTO pdf_annotation_regions(annotation_id,x,y,width,height)
             SELECT ?2,x,y,width,height FROM pdf_excerpt_origin_regions WHERE origin_id=?1 AND ordinal=-1",
            params![origin, annotation],
        )?;
        if source.is_text {
            tx.execute("INSERT INTO pdf_text_marks(annotation_id) VALUES(?1)", [annotation])?;
            tx.execute(
                "INSERT INTO pdf_text_mark_segments(annotation_id,ordinal,x,y,width,height)
                 SELECT ?2,ordinal,x,y,width,height FROM pdf_excerpt_origin_regions WHERE origin_id=?1 AND ordinal>=0",
                params![origin, annotation],
            )?;
        }
        tx.execute("UPDATE pdf_excerpt_origins SET live_annotation=?2 WHERE id=?1", params![origin, annotation])?;
        tx.execute("UPDATE pdf_excerpt_cards SET annotation_id=?2 WHERE origin_id=?1", params![origin, annotation])?;
        tx.commit()?;
        Ok(annotation)
    }

    #[cfg(any(windows, test))]
    pub fn create_pdf_excerpt_card(&self, board: i64, annotation: i64) -> Result<i64> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let source = self.pdf_annotation_hit(annotation)?
            .ok_or("這筆 PDF 標註已移除，或來源筆記在垃圾桶。")?;
        let title = format!("PDF 摘錄 · 第 {} 頁 · {}", source.page, source.attachment_name);
        let body = [source.quote.as_str(), source.comment.as_str()].into_iter()
            .filter(|text| !text.is_empty()).collect::<Vec<_>>().join("\n\n");
        let id = insert(&tx, None, &title, &body)?;
        place_card_on(&tx, board, id)?;
        let origin = snapshot_pdf_source(&tx, annotation)?;
        tx.execute(
            "INSERT INTO pdf_excerpt_cards(note_id,annotation_id,origin_id) VALUES(?1,?2,?3)",
            params![id, annotation, origin],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Build one independent card from a user-selected existing pair.
    #[cfg(any(windows, test))]
    pub(crate) fn create_paired_pdf_excerpt_card(&self, board: i64, annotation: i64, peer: i64) -> Result<i64> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let card = self.write_paired_pdf_excerpt_card(&tx, board, annotation, peer)?;
        tx.commit()?;
        Ok(card)
    }

    /// Save both marks and the card together, so a full board cannot leave half a capture.
    #[cfg(any(windows, test))]
    pub(crate) fn capture_paired_pdf_excerpt_card(
        &self, board: i64, attachment: i64, source: &[u8], pages: [PdfPageText<'_>; 2], comment: &str,
    ) -> Result<i64> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let (first, second) = self.write_pdf_adjacent_text(&tx, attachment, source, pages[0], pages[1], comment)?;
        let card = self.write_paired_pdf_excerpt_card(&tx, board, first, second)?;
        tx.commit()?;
        Ok(card)
    }

    #[cfg(any(windows, test))]
    fn write_paired_pdf_excerpt_card(
        &self, tx: &Transaction<'_>, board: i64, annotation: i64, expected_peer: i64,
    ) -> Result<i64> {
        let stale = "跨頁標註已變更，請重新選取成對標註。";
        let a = self.pdf_annotation_hit(annotation)?.ok_or(stale)?;
        let (peer, peer_page) = self.pdf_adjacent_peer(a.attachment_id, a.id)?
            .filter(|(id, _)| *id == expected_peer).ok_or(stale)?;
        let b = self.pdf_annotation_hit(peer)?.filter(|hit| hit.page == peer_page).ok_or(stale)?;
        let (first, next) = if a.page < b.page { (a, b) } else { (b, a) };
        let first_origin = snapshot_pdf_source(tx, first.id)?;
        let next_origin = snapshot_pdf_source(tx, next.id)?;
        if excerpt_origin_regions(tx, first_origin)?.is_empty() || excerpt_origin_regions(tx, next_origin)?.is_empty() {
            return Err("成對標註缺少任一頁的原選區，無法建立跨頁摘錄。".into());
        }
        let title = format!("PDF 跨頁摘錄 · 第 {}、{} 頁 · {}", first.page, next.page, first.attachment_name);
        let mut body = format!("第 {} 頁\n\n{}\n\n第 {} 頁\n\n{}", first.page, first.quote, next.page, next.quote);
        if !first.comment.is_empty() {
            body.push_str(&format!("\n\n註解（第 {} 頁）\n\n{}", first.page, first.comment));
        }
        if !next.comment.is_empty() && next.comment != first.comment {
            body.push_str(&format!("\n\n註解（第 {} 頁）\n\n{}", next.page, next.comment));
        }
        let card = insert(tx, None, &title, &body)?;
        place_card_on(tx, board, card)?;
        tx.execute(
            "INSERT INTO pdf_excerpt_cards(note_id,annotation_id,origin_id) VALUES(?1,?2,?3)",
            params![card, first.id, first_origin],
        )?;
        tx.execute("INSERT INTO pdf_excerpt_second_page(note_id,origin_id) VALUES(?1,?2)", params![card, next_origin])?;
        Ok(card)
    }

    #[cfg(any(windows, test))]
    pub(crate) fn is_paired_pdf_excerpt(&self, note: i64) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pdf_excerpt_second_page p
             JOIN notes n ON n.id=p.note_id AND n.trash_root IS NULL WHERE p.note_id=?1)",
            [note], |row| row.get(0),
        )?)
    }

    /// `None` means a single-page card. An empty pair means its original PDF is unavailable.
    #[cfg(any(windows, test))]
    pub(crate) fn paired_pdf_excerpt_pages(&self, note: i64) -> Result<Option<Vec<PdfAdjacentExcerptPage>>> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        if !self.is_paired_pdf_excerpt(note)? {
            return Ok(None);
        }
        let anchors = {
            let mut query = tx.prepare(
                "WITH card_origins(ordinal,id) AS (
                    SELECT 0,origin_id FROM pdf_excerpt_cards WHERE note_id=?1
                    UNION ALL
                    SELECT 1,origin_id FROM pdf_excerpt_second_page WHERE note_id=?1
                 ) SELECT o.id,a.id,n.id,o.page,p.id FROM card_origins c
                 JOIN pdf_excerpt_origins o ON o.id=c.id
                 JOIN attachments a ON a.id=o.attachment_id AND a.kind='pdf'
                 JOIN notes n ON n.id=a.note_id AND n.trash_root IS NULL
                 LEFT JOIN pdf_annotations p ON p.id=o.live_annotation AND p.attachment_id=a.id AND p.page=o.page
                 ORDER BY c.ordinal",
            )?;
            query.query_map([note], |row| Ok((row.get::<_, i64>(0)?, PdfAdjacentExcerptPage {
                attachment: row.get(1)?, source_note: row.get(2)?, page: row.get(3)?, annotation: row.get(4)?, focus: Vec::new(),
            })))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let [(_, first), (_, next)] = anchors.as_slice() else { return Ok(Some(Vec::new())) };
        if first.page.checked_add(1) != Some(next.page) || first.attachment != next.attachment {
            return Ok(Some(Vec::new()));
        }
        let mut pages = Vec::with_capacity(2);
        for (origin, mut page) in anchors {
            page.focus = excerpt_origin_regions(&tx, origin)?;
            if page.focus.is_empty() { return Ok(Some(Vec::new())) }
            pages.push(page);
        }
        tx.commit()?;
        Ok(Some(pages))
    }

    #[cfg(any(windows, test))]
    pub fn is_pdf_excerpt(&self, note: i64) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pdf_excerpt_cards c
             JOIN notes n ON n.id=c.note_id AND n.trash_root IS NULL WHERE c.note_id=?1)",
            [note], |row| row.get(0),
        )?)
    }

    #[cfg(any(windows, test))]
    pub fn pdf_excerpt_source(&self, note: i64) -> Result<Option<PdfAnnotationHit>> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)?;
        let annotation = tx.query_row(
            "SELECT c.annotation_id FROM pdf_excerpt_cards c
             JOIN notes n ON n.id=c.note_id AND n.trash_root IS NULL WHERE c.note_id=?1",
            [note], |row| row.get::<_, Option<i64>>(0),
        ).optional()?.flatten();
        let hit = match annotation {
            Some(id) => self.pdf_annotation_hit(id)?,
            None => None,
        };
        tx.commit()?;
        Ok(hit)
    }

    #[cfg(any(windows, test))]
    pub fn pdf_excerpt_cards_for(&self, annotation: i64) -> Result<Vec<(i64, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT n.id,n.title FROM pdf_excerpt_cards c
             JOIN notes n ON n.id=c.note_id AND n.trash_root IS NULL
             JOIN pdf_annotations p ON p.id=?1
             JOIN attachments a ON a.id=p.attachment_id AND a.kind='pdf'
             JOIN notes source ON source.id=a.note_id AND source.trash_root IS NULL
             WHERE c.annotation_id=?1 OR EXISTS(
                SELECT 1 FROM pdf_excerpt_second_page second
                JOIN pdf_excerpt_origins o ON o.id=second.origin_id AND o.live_annotation=p.id
                WHERE second.note_id=c.note_id AND o.attachment_id=p.attachment_id AND o.page=p.page
             ) ORDER BY n.id",
        )?;
        let rows = statement.query_map([annotation], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    #[cfg(any(windows, test))]
    pub fn pdf_excerpt_cards_on_board(&self, board: i64) -> Result<Vec<(i64, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT n.id,n.title FROM board_cards b
             JOIN pdf_excerpt_cards c ON c.note_id=b.note_id
             JOIN notes n ON n.id=c.note_id AND n.trash_root IS NULL
             WHERE b.board_id=?1 ORDER BY n.id",
        )?;
        let rows = statement.query_map([board], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_card_library_filters_tags_types_and_reuses_mixed_cards_after_reopen() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let database = dir.join("workspace.sqlite");
        let backup = dir.join("copy.sqlite");
        let pdf = dir.join("來源.pdf");
        std::fs::write(&pdf, include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf"))?;
        let mut db = Notebook::open(&database, false)?;
        let source_board = db.create_board("原白板")?;
        let board = db.create_board("整理白板")?;
        let first = db.add(None, "同名鏡頭", "---\ntags: [電影/夜景]\n---\n甲段獨有的文字。")?;
        let second = db.add(None, "同名鏡頭", "乙段獨有的文字。#電影/夜景/試拍\n\n`#電影/夜景`")?;
        let third = db.add(None, "內文查找", &format!("{}尾端可搜尋 #電影/夜景", "甲".repeat(5000)))?;
        let source_note = db.add(None, "原 PDF", "")?;
        let attachment = db.import_attachment(source_note, &pdf)?;
        let mark = db.add_pdf_annotation_with_region(attachment, 2, "引用原文", "",
            super::super::PdfRegion { x: 0.12, y: 0.24, width: 0.35, height: 0.08 })?;
        let excerpt = db.create_pdf_excerpt_card(source_board, mark)?;
        db.add_card_on(board, first)?;
        db.move_card_on(board, first, 410, 522)?;

        let search = |pdf_only, starred, on_board, text, tag| db.board_card_library(board, BoardCardLibraryFilter {
            pdf: PdfExcerptLibraryFilter { attachment: None, unavailable_only: false, on_board, page: None, text },
            pdf_only, starred, tag,
        });
        let exact = search(Some(false), None, None, "", "#電影/夜景")?.0;
        assert_eq!(exact.iter().map(|hit| hit.card).collect::<Vec<_>>(), [third, first]);
        assert!(exact[0].body.ends_with("…（預覽節錄）"));
        assert!(exact[1].body.contains("甲段獨有"));
        assert!(exact[1].on_board);
        assert!(exact.iter().all(|hit| !hit.is_pdf && hit.source.is_none()));
        let text = search(Some(false), None, Some(false), "尾端可搜尋", "#電影/夜景")?.0;
        assert_eq!(text[0].card, third);
        assert_eq!(search(Some(false), None, None, "乙段獨有", "")?.0[0].card, second);
        let pdfs = search(Some(true), None, Some(false), "引用原文", "")?.0;
        assert_eq!((pdfs[0].card, pdfs[0].pages), (excerpt, [Some(2), None]));
        assert_eq!(pdfs[0].source.as_ref().unwrap().attachment, attachment);
        assert!(db.place_existing_pdf_excerpts(board, &[first]).is_err());
        assert_eq!(db.place_board_library_cards(board, &[first, excerpt, third])?, 2);
        assert_eq!(db.place_board_library_cards(board, &[first, excerpt, third])?, 0);
        assert_eq!(db.set_board_library_stars(&[first, excerpt], true)?, 2);
        assert_eq!(db.set_board_library_stars(&[first, excerpt], true)?, 0);
        assert_eq!(search(None, Some(true), Some(true), "", "")?.0.iter().map(|hit| hit.card).collect::<Vec<_>>(), [excerpt, first]);
        db.backup(&backup)?;
        drop(db);

        let reopened = Notebook::open(&backup, false)?;
        let cards = reopened.cards_on(board)?;
        assert_eq!(cards.iter().map(|c| c.id).collect::<Vec<_>>(), [first, third, excerpt]);
        assert_eq!((cards[0].x, cards[0].y), (410, 522));
        assert!(reopened.note(first)?.unwrap().row.pinned);
        assert!(reopened.note(excerpt)?.unwrap().row.pinned);
        assert_eq!(reopened.pdf_excerpt_source(excerpt)?.unwrap().id, mark);
        assert_eq!(reopened.set_board_library_stars(&[first, excerpt], false)?, 2);
        assert!(!reopened.note(excerpt)?.unwrap().row.pinned);
        drop(reopened);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn board_card_library_mixed_add_and_star_roll_back_on_a_stale_second_id() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let mut db = Notebook::open(&dir.join("workspace.sqlite"), false)?;
        let board = db.create_board("收集白板")?;
        let first = db.add(None, "普通卡", "")?;
        let second = db.add(None, "舊 PDF 卡", "")?;
        db.connection.execute("INSERT INTO pdf_excerpt_cards(note_id) VALUES(?1)", [second])?;
        db.trash(second)?;
        assert!(db.place_board_library_cards(board, &[first, second]).is_err());
        assert!(db.cards_on(board)?.is_empty());
        assert!(db.set_board_library_stars(&[first, second], true).is_err());
        assert!(!db.note(first)?.unwrap().row.pinned);
        assert!(db.place_board_library_cards(board, &[first, first]).is_err());
        assert!(db.set_board_library_stars(&[first, first], true).is_err());
        db.restore(second)?;
        assert_eq!(db.place_board_library_cards(board, &[first, second])?, 2);
        assert_eq!(db.set_board_library_stars(&[first, second], true)?, 2);
        assert!(db.note(second)?.unwrap().row.pinned);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn excerpt_library_filters_exact_same_named_sources_scopes_and_paired_pages() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("notes.sqlite");
        let backup = dir.join("backup.sqlite");
        let pdf = dir.join("同名原稿.pdf");
        let bytes = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
        std::fs::write(&pdf, bytes)?;
        let mut db = Notebook::open(&path, false)?;
        let source_board = db.create_board("來源白板")?;
        let target = db.create_board("批次目的白板")?;
        let left_note = db.add(None, "甲來源", "")?;
        let right_note = db.add(None, "乙來源", "")?;
        let left = db.import_attachment(left_note, &pdf)?;
        let right = db.import_attachment(right_note, &pdf)?;
        let regions = [super::super::PdfRegion { x: 0.12, y: 0.24, width: 0.35, height: 0.08 }];
        let pages = [PdfPageText { page: 1, quote: "甲的第一頁", regions: &regions },
            PdfPageText { page: 2, quote: "甲的第二頁", regions: &regions }];
        let (first, next) = db.add_pdf_adjacent_text(left, bytes, pages[0], pages[1], "甲註解")?;
        let simple = db.create_pdf_excerpt_card(source_board, first)?;
        let pair = db.create_paired_pdf_excerpt_card(source_board, first, next)?;
        let other_mark = db.add_pdf_annotation_with_region(right, 2, "乙的第二頁", "乙註解", regions[0])?;
        let snapshot = db.create_pdf_excerpt_card(source_board, other_mark)?;
        db.remove_pdf_annotation(other_mark)?;
        let legacy = db.add(None, "舊卡", &format!("{}原始舊卡全文詞", "紙".repeat(5000)))?;
        db.connection.execute("INSERT INTO pdf_excerpt_cards(note_id) VALUES(?1)", [legacy])?;
        assert!(db.place_existing_pdf_excerpt(target, simple)?);
        db.move_card_on(target, simple, 602, 703)?;

        let sources = db.pdf_excerpt_library_sources()?;
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().any(|s| s.attachment == left && s.note == left_note && s.file == "同名原稿.pdf"));
        assert!(sources.iter().any(|s| s.attachment == right && s.note == right_note && s.file == "同名原稿.pdf"));
        let find = |attachment, unavailable_only, on_board, page, text| db.pdf_excerpt_library(target,
            PdfExcerptLibraryFilter { attachment, unavailable_only, on_board, page, text });
        let results = find(Some(left), false, None, Some(2), "")?.0;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].card, pair);
        assert_eq!(results[0].pages, [Some(1), Some(2)]);
        assert_eq!(results[0].quotes, ["甲的第一頁", "甲的第二頁"]);
        assert_eq!(results[0].live, [true, true]);
        let results = find(Some(right), false, None, Some(2), "")?.0;
        assert_eq!(results[0].card, snapshot);
        assert_eq!(results[0].live, [false, false]);
        assert_eq!(results[0].source.as_ref().unwrap().attachment, right);
        assert_eq!(results[0].quotes[0], "乙的第二頁");
        assert_eq!(find(Some(left), false, Some(true), None, "")?.0[0].card, simple);
        assert_eq!(find(Some(left), false, Some(false), None, "")?.0[0].card, pair);
        let results = find(None, true, None, None, "原始舊卡全文詞")?.0;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].card, legacy);
        assert!(results[0].source.is_none());
        assert!(results[0].body.ends_with("…（預覽節錄）"));
        assert!(!results[0].body.contains("原始舊卡全文詞"));
        assert!(results[0].body.chars().count() < 4120);

        db.remove_attachment(right)?;
        assert!(find(Some(right), false, None, None, "")?.0.is_empty());
        assert_eq!(find(None, true, None, Some(2), "")?.0[0].card, snapshot);
        assert_eq!(db.place_existing_pdf_excerpts(target, &[simple, pair, legacy, snapshot])?, 3);
        assert_eq!(db.place_existing_pdf_excerpts(target, &[simple, pair, legacy, snapshot])?, 0);
        db.backup(&backup)?;
        drop(db);

        let reopened = Notebook::open(&backup, false)?;
        let card = &reopened.cards_on(target)?[0];
        assert_eq!((card.id, card.x, card.y), (simple, 602, 703));
        assert_eq!(reopened.pdf_excerpt_cards_on_board(target)?.len(), 4);
        let anchors = reopened.paired_pdf_excerpt_pages(pair)?.unwrap();
        assert_eq!((anchors[0].annotation, anchors[1].annotation), (Some(first), Some(next)));
        assert!(reopened.pdf_excerpt_source(snapshot)?.is_none());
        assert!(reopened.note(legacy)?.unwrap().body.ends_with("原始舊卡全文詞"));
        drop(reopened);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn excerpt_library_batch_never_leaves_a_prefix_when_a_card_is_no_longer_placeable() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let mut db = Notebook::open(&dir.join("notes.sqlite"), false)?;
        let target = db.create_board("批次目的白板")?;
        let first = db.add(None, "摘錄甲", "第一張")?;
        let second = db.add(None, "摘錄乙", "第二張")?;
        for note in [first, second] {
            db.connection.execute("INSERT INTO pdf_excerpt_cards(note_id) VALUES(?1)", [note])?;
        }
        assert!(db.place_existing_pdf_excerpts(target, &[first, first]).is_err());
        db.trash(second)?;
        assert!(db.place_existing_pdf_excerpts(target, &[first, second]).is_err());
        assert!(db.cards_on(target)?.is_empty());
        db.restore(second)?;
        let injected = format!("CREATE TEMP TRIGGER deny_second_excerpt BEFORE INSERT ON board_cards
                               WHEN NEW.note_id={second} BEGIN SELECT RAISE(ABORT,'synthetic full board'); END;");
        db.connection.execute_batch(&injected)?;
        assert!(db.place_existing_pdf_excerpts(target, &[first, second]).is_err());
        assert!(db.cards_on(target)?.is_empty());
        db.connection.execute_batch("DROP TRIGGER deny_second_excerpt")?;
        assert_eq!(db.place_existing_pdf_excerpts(target, &[first, second])?, 2);
        assert_eq!(db.cards_on(target)?.iter().map(|card| card.id).collect::<Vec<_>>(), [first, second]);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn paired_excerpt_reopens_with_both_exact_pages_and_keeps_shared_origins() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("v14.sqlite");
        let backup = dir.join("backup.sqlite");
        let pdf = dir.join("同名原稿.pdf");
        let bytes = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
        std::fs::write(&pdf, bytes)?;
        let mut db = Notebook::open(&path, false)?;
        let board = db.create_board("跨頁來源板")?;
        let destination = db.create_board("第二張白板")?;
        let source_note = db.add(None, "原稿", "")?;
        let alias_note = db.add(None, "同名原稿", "")?;
        let attachment = db.import_attachment(source_note, &pdf)?;
        let alias_attachment = db.import_attachment(alias_note, &pdf)?;
        let regions = [super::super::PdfRegion { x: 0.22, y: 0.74, width: 0.31, height: 0.04 },
            super::super::PdfRegion { x: 0.09, y: 0.18, width: 0.28, height: 0.06 }];
        let first = PdfPageText { page: 1, quote: "第一頁引文", regions: &regions[..1] };
        let second = PdfPageText { page: 2, quote: "第二頁的唯一詞", regions: &regions[1..] };
        let [a, b] = [first, second];
        let (first_id, second_id) = db.add_pdf_adjacent_text(attachment, bytes, a, b, "共同註解")?;
        let (_, alias_second) = db.add_pdf_adjacent_text(alias_attachment, bytes, a, b, "同名註解")?;
        let single = db.create_pdf_excerpt_card(board, second_id)?;
        super::super::tests::remove_v15_schema(&db.connection)?;
        db.connection.pragma_update(None, "user_version", 14)?;
        drop(db);

        let db = Notebook::open(&path, false)?;
        assert!(db.paired_pdf_excerpt_pages(single)?.is_none());
        assert!(db.create_paired_pdf_excerpt_card(board, first_id, alias_second).is_err());
        assert_eq!(db.pdf_excerpt_cards_on_board(board)?.len(), 1);
        let pair = db.create_paired_pdf_excerpt_card(board, second_id, first_id)?;
        assert!(db.place_existing_pdf_excerpt(destination, pair)?);
        assert_eq!(db.pdf_excerpt_cards_for(first_id)?[0].0, pair);
        assert_eq!(db.pdf_excerpt_cards_for(second_id)?.iter().map(|card| card.0).collect::<Vec<_>>(), [single, pair]);
        assert_eq!(db.search_pdf_excerpt_cards("第二頁的唯一詞")?.iter().map(|card| card.0).collect::<Vec<_>>(), [pair, single]);
        let text = db.note(pair)?.unwrap().body;
        assert_eq!(text.matches("共同註解").count(), 1);
        assert!(text.find(first.quote).unwrap() < text.find(second.quote).unwrap());
        db.backup(&backup)?;
        drop(db);

        let db = Notebook::open(&backup, false)?;
        let recorded = db.paired_pdf_excerpt_pages(pair)?.unwrap();
        assert_eq!(recorded.iter().map(|p| (p.attachment, p.source_note, p.page, p.annotation)).collect::<Vec<_>>(),
            [(attachment, source_note, 1, Some(first_id)), (attachment, source_note, 2, Some(second_id))]);
        assert_eq!(recorded[0].focus, first.regions);
        assert_eq!(recorded[1].focus, second.regions);
        assert_eq!(db.pdf_excerpt_cards_on_board(destination)?[0].0, pair);
        db.remove_pdf_annotation(second_id)?;
        db.remove_pdf_annotation(first_id)?;
        let recorded = db.paired_pdf_excerpt_pages(pair)?.unwrap();
        assert!(recorded.iter().all(|p| p.annotation.is_none()));
        assert_eq!(recorded[0].focus, first.regions);
        assert_eq!(recorded[1].focus, second.regions);
        assert!(db.pdf_excerpt_recovery_preview(pair)?.is_none());
        assert_eq!(db.list_pdf_annotations(attachment)?.len(), 0);
        let single_recovery = db.pdf_excerpt_recovery_preview(single)?.unwrap();
        let recreated = db.restore_pdf_excerpt_annotation(single, single_recovery.origin)?;
        assert_eq!(db.paired_pdf_excerpt_pages(pair)?.unwrap()[1].annotation, Some(recreated));

        db.remove_attachment(attachment)?;
        assert!(db.paired_pdf_excerpt_pages(pair)?.unwrap().is_empty());
        assert_eq!(db.note(pair)?.unwrap().body, text);
        assert!(db.paired_pdf_excerpt_pages(single)?.is_none());
        db.remove_card_on(board, pair)?;
        db.remove_card_on(destination, pair)?;
        db.connection.execute("DELETE FROM notes WHERE id=?1", [pair])?;
        let retained: Vec<i64> = {
            let mut query = db.connection.prepare("SELECT id FROM pdf_excerpt_origins ORDER BY id")?;
            query.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        assert_eq!(retained, [single_recovery.origin]);
        assert_eq!(db.connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?, "ok");
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn direct_paired_capture_rolls_back_marks_and_card_on_failure() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let pdf = dir.join("來源.pdf");
        let bytes = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
        std::fs::write(&pdf, bytes)?;
        let mut db = Notebook::open(&dir.join("workspace.sqlite"), false)?;
        let board = db.create_board("直存跨頁白板")?;
        let note = db.add(None, "來源", "")?;
        let attachment = db.import_attachment(note, &pdf)?;
        let region = [super::super::PdfRegion { x: 0.1, y: 0.1, width: 0.2, height: 0.1 }];
        let pages = [PdfPageText { page: 1, quote: "第一頁", regions: &region },
            PdfPageText { page: 2, quote: "第二頁", regions: &region }];
        assert!(db.capture_paired_pdf_excerpt_card(board, attachment, b"%PDF-2.0\n", pages, "").is_err());
        assert!(db.capture_paired_pdf_excerpt_card(i64::MAX, attachment, bytes, pages, "").is_err());
        db.connection.execute_batch("CREATE TEMP TRIGGER reject_pair_card BEFORE INSERT ON pdf_excerpt_second_page
            BEGIN SELECT RAISE(ABORT,'synthetic pair card failure'); END;")?;
        assert!(db.capture_paired_pdf_excerpt_card(board, attachment, bytes, pages, "").is_err());
        assert!(db.list_pdf_annotations(attachment)?.is_empty());
        assert!(db.cards_on(board)?.is_empty());
        assert_eq!(db.rows(super::super::View::All, "")?.len(), 1);
        db.connection.execute_batch("DROP TRIGGER reject_pair_card")?;
        let card = db.capture_paired_pdf_excerpt_card(board, attachment, bytes, pages, "有效")?;
        assert_eq!(db.cards_on(board)?[0].id, card);
        let pages = db.paired_pdf_excerpt_pages(card)?.unwrap();
        let first = pages[0].annotation.unwrap();
        let second = pages[1].annotation.unwrap();
        assert_eq!(db.pdf_adjacent_peer(attachment, first)?, Some((second, 2)));
        assert_eq!(db.pdf_adjacent_peer(attachment, second)?, Some((first, 1)));
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn legacy_excerpts_recover_the_same_original_geometry_and_never_guess_an_attachment() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("v12.sqlite3");
        let pdf = dir.join("原稿.pdf");
        let backup = dir.join("reopen.sqlite3");
        std::fs::write(&pdf, b"%PDF-2.0\n")?;
        let mut db = Notebook::open(&path, false)?;
        let board = db.create_board("復原白板")?;
        let source = db.add(None, "來源", "")?;
        let wrong = db.add(None, "來源", "")?;
        let attachment = db.import_attachment(source, &pdf)?;
        let other_attachment = db.import_attachment(wrong, &pdf)?;
        let regions = [
            super::super::PdfRegion { x: 0.12, y: 0.16, width: 0.34, height: 0.04 },
            super::super::PdfRegion { x: 0.07, y: 0.28, width: 0.27, height: 0.05 },
        ];
        let original = db.add_pdf_text_mark_segments(attachment, 2, "原文 B\n第二行", "註解", &regions)?;
        let old_orphan_annotation = db.add_pdf_annotation(attachment, 3, "早已移除", "")?;
        let other = db.add_pdf_text_mark_segments(other_attachment, 2, "原文 B\n第二行", "註解", &regions)?;
        let card = db.create_pdf_excerpt_card(board, original)?;
        let sibling = db.create_pdf_excerpt_card(board, original)?;
        let old_orphan = db.create_pdf_excerpt_card(board, old_orphan_annotation)?;
        db.remove_pdf_annotation(old_orphan_annotation)?;
        db.save(card, 1, "我的卡片", "另寫的內容")?;
        super::super::tests::remove_v13_schema(&db.connection)?;
        db.connection.pragma_update(None, "user_version", 12)?;
        drop(db);

        let db = Notebook::open(&path, false)?;
        assert_eq!(db.search_pdf_excerpt_cards("原文 b")?.iter().map(|row| row.0).collect::<Vec<_>>(), [sibling, card]);
        assert_eq!(db.search_pdf_excerpt_cards("註解")?.iter().map(|row| row.0).collect::<Vec<_>>(), [sibling, card]);
        assert!(db.pdf_excerpt_recovery_preview(card)?.is_none());
        assert!(db.pdf_excerpt_recovery_preview(old_orphan)?.is_none());
        assert!(db.pdf_excerpt_snapshot(card)?.is_none());
        assert!(db.pdf_excerpt_snapshot(old_orphan)?.is_none());
        db.remove_pdf_annotation(original)?;
        db.backup(&backup)?;
        drop(db);

        let mut db = Notebook::open(&backup, false)?;
        let preview = db.pdf_excerpt_recovery_preview(card)?.ok_or("missing recovery")?;
        assert_eq!((preview.name.as_str(), preview.page, preview.quote.as_str(), preview.comment.as_str()),
            ("原稿.pdf", 2, "原文 B\n第二行", "註解"));
        assert_eq!(db.pdf_excerpt_recovery_preview(sibling)?.unwrap().origin, preview.origin);
        let snapshot = db.pdf_excerpt_snapshot(card)?.ok_or("missing read-only preview")?;
        assert_eq!((snapshot.note, snapshot.attachment, snapshot.page), (source, attachment, 2));
        assert_eq!(snapshot.focus, regions);
        assert_eq!(db.pdf_excerpt_snapshot(sibling)?.unwrap().focus, regions);
        assert!(db.pdf_excerpt_source(card)?.is_none());
        assert!(db.list_pdf_annotations(attachment)?.is_empty());
        db.trash(source)?;
        assert!(db.pdf_excerpt_snapshot(card)?.is_none());
        assert!(db.restore_pdf_excerpt_annotation(card, preview.origin).is_err());
        db.restore(source)?;
        assert!(db.restore_pdf_excerpt_annotation(card, preview.origin + 1).is_err());
        let restored = db.restore_pdf_excerpt_annotation(card, preview.origin)?;
        assert_ne!(original, restored);
        assert!(db.restore_pdf_excerpt_annotation(sibling, preview.origin).is_err());
        assert_eq!(db.pdf_excerpt_source(card)?.unwrap().attachment_id, attachment);
        assert_eq!(db.pdf_excerpt_source(sibling)?.unwrap().id, restored);
        assert!(db.pdf_excerpt_snapshot(card)?.is_none());
        assert_eq!(db.pdf_annotation_focus_regions(attachment, 2, restored)?, regions);
        assert_eq!(db.pdf_annotation_hit(other)?.unwrap().attachment_id, other_attachment);
        assert_eq!(db.note(card)?.unwrap().body, "另寫的內容");

        db.remove_attachment(attachment)?;
        assert!(db.pdf_excerpt_recovery_preview(card)?.is_none());
        assert!(db.pdf_excerpt_snapshot(card)?.is_none());
        assert!(db.restore_pdf_excerpt_annotation(card, preview.origin).is_err());
        db.remove_card_on(board, card)?;
        db.connection.execute("DELETE FROM notes WHERE id=?1", [card])?;
        assert!(db.connection.query_row("SELECT EXISTS(SELECT 1 FROM pdf_excerpt_origins)", [], |row| row.get::<_, bool>(0))?);
        db.remove_card_on(board, sibling)?;
        db.connection.execute("DELETE FROM notes WHERE id=?1", [sibling])?;
        assert!(!db.connection.query_row("SELECT EXISTS(SELECT 1 FROM pdf_excerpt_origins)", [], |row| row.get::<_, bool>(0))?);
        assert_eq!(db.connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?, "ok");
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn rendered_pdf_highlight_board_card_reopens_on_the_same_source_pixels() -> Result<()> {
        use crate::pdf_render::render_page;
        use std::fs;

        let dir = super::super::tests::data_dir();
        let workspace = dir.join("實際摘錄.sqlite3");
        let input = dir.join("剪輯範圍.pdf");
        let pdf = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
        fs::write(&input, pdf)?;
        let page = 2;
        let raster = render_page(pdf.to_vec(), page)?;
        let selection = raster.text.select([0.43, 0.639], [0.89, 0.639]).ok_or("missing fixture selection")?;
        assert_eq!(selection.quote, "Synthetic shot; selection B");
        let expected = raster.highlighted_bgra(&selection.regions).ok_or("missing highlight")?;
        assert_ne!(expected, raster.bgra);

        let (board, source, attachment, annotation, card) = {
            let mut db = Notebook::open(&workspace, true)?;
            let board = db.create_board("版面摘錄")?;
            let source = db.add(None, "原稿", "")?;
            let duplicate = db.add(None, "同名副本", "")?;
            let attachment = db.import_attachment(source, &input)?;
            let other = db.import_attachment(duplicate, &input)?;
            assert_ne!(other, attachment);
            let (_, source_bytes) = db.read_pdf_attachment(attachment)?;
            let mark = PdfPageText { page, quote: &selection.quote, regions: &selection.regions };
            assert!(db.add_pdf_text_from_source(attachment, b"stale viewer bytes", mark, "不能保存").is_err());
            assert!(db.list_pdf_annotations(attachment)?.is_empty());
            let annotation = db.add_pdf_text_from_source(attachment, &source_bytes, mark, "保留原稿來源")?;
            let card = db.create_pdf_excerpt_card(board, annotation)?;
            (board, source, attachment, annotation, card)
        };
        fs::remove_file(&input)?;
        {
            let db = Notebook::open(&workspace, true)?;
            let board_cards = db.pdf_excerpt_cards_on_board(board)?;
            assert_eq!(board_cards.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [card]);
            let hit = db.pdf_excerpt_source(card)?.ok_or("excerpt did not return to a source")?;
            assert_eq!((hit.note_id, hit.attachment_id, hit.page, hit.id), (source, attachment, page, annotation));
            assert_eq!(hit.quote, selection.quote);
            let regions = db.pdf_annotation_focus_regions(hit.attachment_id, hit.page, hit.id)?;
            assert_eq!(regions, selection.regions);
            let (_, restored_bytes) = db.read_pdf_attachment(hit.attachment_id)?;
            assert_eq!(restored_bytes.as_slice(), pdf);
            let restored = render_page(restored_bytes, hit.page)?;
            assert_eq!(restored.highlighted_bgra(&regions).as_deref(), Some(expected.as_slice()));

            db.remove_attachment(attachment)?;
            assert!(db.pdf_excerpt_source(card)?.is_none());
            assert!(db.pdf_excerpt_snapshot(card)?.is_none());
            assert!(db.note(card)?.unwrap().body.contains(&selection.quote));
        }
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn excerpt_cards_are_atomic_independent_notes_with_stable_pdf_sources() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let db_path = dir.join("notes.sqlite3");
        let attachment_path = dir.join("原稿.pdf");
        let backup = dir.join("backup.sqlite3");
        std::fs::write(&attachment_path, b"%PDF-2.0\n")?;
        let mut db = Notebook::open(&db_path, false)?;
        let board = db.create_board("摘錄白板")?;
        let another = db.create_board("第二張摘錄白板")?;
        let source = db.add(None, "同名來源", "")?;
        let other = db.add(None, "同名來源", "")?;
        let attachment = db.import_attachment(source, &attachment_path)?;
        let other_attachment = db.import_attachment(other, &attachment_path)?;
        let quote = "保留此行\nSecond line";
        let segments = [
            crate::store::PdfRegion { x: 0.12, y: 0.16, width: 0.34, height: 0.04 },
            crate::store::PdfRegion { x: 0.07, y: 0.28, width: 0.27, height: 0.05 },
        ];
        let original = db.add_pdf_text_mark_segments(attachment, 2, quote, "來源批註", &segments)?;
        let adjacent = db.add_pdf_text_mark_segments(attachment, 2, quote, "來源批註", &segments)?;
        let other_annotation = db.add_pdf_text_mark_segments(other_attachment, 2, quote, "來源批註", &segments)?;
        let oversized_board = db.create_board("已滿白板")?;
        for index in 0..128 {
            let note = db.add(None, &format!("已放入 {index}"), "")?;
            db.add_card_on(oversized_board, note)?;
        }
        let baseline = db.rows(crate::store::View::All, "")?.len();
        assert!(db.create_pdf_excerpt_card(oversized_board, original).is_err());
        assert!(db.create_pdf_excerpt_card(i64::MAX, original).is_err());
        db.connection.execute_batch(
            "CREATE TEMP TRIGGER excerpt_failure BEFORE INSERT ON pdf_excerpt_cards
             BEGIN SELECT RAISE(ABORT,'synthetic excerpt failure'); END;",
        )?;
        assert!(db.create_pdf_excerpt_card(board, original).is_err());
        assert!(db.cards_on(board)?.is_empty());
        assert_eq!(db.rows(crate::store::View::All, "")?.len(), baseline);
        assert!(db.pdf_excerpt_cards_for(original)?.is_empty());
        db.connection.execute_batch("DROP TRIGGER excerpt_failure")?;

        let excerpt = db.create_pdf_excerpt_card(board, original)?;
        assert!(db.is_pdf_excerpt(excerpt)?);
        assert!(db.cards_on(1)?.is_empty());
        assert_eq!(db.pdf_excerpt_cards_on_board(board)?[0].0, excerpt);
        let saved = db.note(excerpt)?.ok_or("missing independent excerpt")?;
        assert_eq!(saved.body, format!("{quote}\n\n來源批註"));
        db.save(excerpt, saved.version, "獨立改名", "我自己的筆記")?;
        assert!(db.place_existing_pdf_excerpt(oversized_board, excerpt).is_err());
        assert!(db.place_existing_pdf_excerpt(i64::MAX, excerpt).is_err());
        assert!(db.place_existing_pdf_excerpt(another, source).is_err());
        assert!(db.place_existing_pdf_excerpt(another, excerpt)?);
        db.move_card_on(another, excerpt, 225, 306)?;
        assert!(!db.place_existing_pdf_excerpt(another, excerpt)?);
        assert_eq!(db.rows(crate::store::View::All, "")?.len(), baseline + 1);
        assert_eq!((db.cards_on(another)?[0].x, db.cards_on(another)?[0].y), (225, 306));
        assert_eq!(db.pdf_excerpt_cards_for(original)?, [(excerpt, "獨立改名".into())]);
        assert!(db.pdf_excerpt_cards_for(adjacent)?.is_empty());
        assert!(db.pdf_excerpt_cards_for(other_annotation)?.is_empty());
        db.trash(source)?;
        assert!(db.pdf_excerpt_source(excerpt)?.is_none());
        assert!(db.create_pdf_excerpt_card(board, original).is_err());
        assert_eq!(db.note(excerpt)?.unwrap().body, "我自己的筆記");
        db.restore(source)?;
        db.backup(&backup)?;
        drop(db);

        let reopened = Notebook::open(&backup, false)?;
        let hit = reopened.pdf_excerpt_source(excerpt)?.ok_or("missing original source")?;
        assert_eq!((hit.note_id, hit.attachment_id, hit.page, hit.id), (source, attachment, 2, original));
        assert_eq!(hit.quote, quote);
        assert_eq!(reopened.pdf_annotation_focus_regions(attachment, 2, original)?, segments);
        assert_eq!(reopened.pdf_excerpt_cards_on_board(another)?, [(excerpt, "獨立改名".into())]);
        assert_eq!((reopened.cards_on(another)?[0].x, reopened.cards_on(another)?[0].y), (225, 306));
        reopened.remove_card_on(another, excerpt)?;
        assert!(reopened.pdf_excerpt_cards_on_board(another)?.is_empty());
        assert_eq!(reopened.pdf_excerpt_source(excerpt)?.unwrap().id, original);
        reopened.remove_pdf_annotation(original)?;
        assert!(reopened.place_existing_pdf_excerpt(another, excerpt)?);
        assert_eq!(reopened.pdf_excerpt_snapshot(excerpt)?.unwrap().focus, segments);
        assert!(reopened.is_pdf_excerpt(excerpt)?);
        assert!(reopened.pdf_excerpt_source(excerpt)?.is_none());
        assert_eq!(reopened.pdf_excerpt_cards_on_board(board)?, [(excerpt, "獨立改名".into())]);
        assert_eq!(reopened.note(excerpt)?.unwrap().body, "我自己的筆記");
        assert!(reopened.pdf_excerpt_cards_for(adjacent)?.is_empty());
        reopened.trash(excerpt)?;
        assert!(reopened.place_existing_pdf_excerpt(board, excerpt).is_err());
        assert!(reopened.pdf_excerpt_snapshot(excerpt)?.is_none());
        drop(reopened);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
