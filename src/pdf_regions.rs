use super::{Notebook, Result};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

pub(super) const SCHEMA: &str = "
    CREATE TABLE pdf_annotation_regions(
        annotation_id INTEGER PRIMARY KEY REFERENCES pdf_annotations(id) ON DELETE CASCADE,
        x REAL NOT NULL CHECK(x>=0 AND x<1),
        y REAL NOT NULL CHECK(y>=0 AND y<1),
        width REAL NOT NULL CHECK(width>0 AND x+width<=1),
        height REAL NOT NULL CHECK(height>0 AND y+height<=1)
    );
";

pub(super) const TEXT_MARKS_SCHEMA: &str = "
    CREATE TABLE pdf_text_marks(
        annotation_id INTEGER PRIMARY KEY REFERENCES pdf_annotation_regions(annotation_id) ON DELETE CASCADE
    );
";

pub(super) const TEXT_SEGMENTS_SCHEMA: &str = "
    CREATE TABLE pdf_text_mark_segments(
        annotation_id INTEGER NOT NULL REFERENCES pdf_text_marks(annotation_id) ON DELETE CASCADE,
        ordinal INTEGER NOT NULL CHECK(ordinal>=0 AND ordinal<64),
        x REAL NOT NULL CHECK(x>=0 AND x<1),
        y REAL NOT NULL CHECK(y>=0 AND y<1),
        width REAL NOT NULL CHECK(width>0 AND x+width<=1),
        height REAL NOT NULL CHECK(height>0 AND y+height<=1),
        PRIMARY KEY(annotation_id,ordinal)
    ) WITHOUT ROWID;
";

pub(super) const ADJACENT_SCHEMA: &str = "
    CREATE TABLE pdf_text_adjacent_pairs(
        first_id INTEGER PRIMARY KEY REFERENCES pdf_annotations(id) ON DELETE CASCADE,
        next_id INTEGER NOT NULL UNIQUE REFERENCES pdf_annotations(id) ON DELETE CASCADE,
        CHECK(first_id <> next_id)
    ) WITHOUT ROWID;
";

#[cfg(any(windows, test))]
#[derive(Clone, Copy)]
pub(crate) struct PdfPageText<'a> {
    pub page: u32,
    pub quote: &'a str,
    pub regions: &'a [PdfRegion],
}

/// Fractions of the whole page, starting at its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdfRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PdfRegion {
    pub const MAX_TEXT_SEGMENTS: usize = 64;
    fn validate(self) -> Result<()> {
        if ![self.x, self.y, self.width, self.height].into_iter().all(f64::is_finite)
            || !(0.0..1.0).contains(&self.x)
            || !(0.0..1.0).contains(&self.y)
            || self.width <= 0.0
            || self.height <= 0.0
            || self.x + self.width > 1.0
            || self.y + self.height > 1.0
        {
            return Err("範圍座標須以左上角為原點，使用 0 到 1 之間的頁面比例，且寬高須大於 0。".into());
        }
        Ok(())
    }
}

fn text_regions_in_reading_order(regions: &[PdfRegion]) -> bool {
    let wrap = regions.windows(2).position(|pair| pair[0].y + pair[0].height > pair[1].y);
    let Some(wrap) = wrap else { return true };
    let (left, right) = regions.split_at(wrap + 1);
    let left_edge = left.iter().map(|r| r.x + r.width).fold(0.0, f64::max);
    let right_edge = right.iter().map(|r| r.x).fold(1.0, f64::min);
    left_edge < right_edge && !right.windows(2).any(|pair| pair[0].y + pair[0].height > pair[1].y)
}

impl Notebook {
    pub fn pdf_region(&self, annotation: i64) -> Result<Option<PdfRegion>> {
        Ok(self.connection.query_row(
            "SELECT r.x,r.y,r.width,r.height FROM pdf_annotation_regions r
             JOIN pdf_annotations p ON p.id=r.annotation_id
             JOIN attachments a ON a.id=p.attachment_id
             JOIN notes n ON n.id=a.note_id
             WHERE p.id=?1 AND a.kind='pdf' AND n.trash_root IS NULL",
            [annotation],
            |row| Ok(PdfRegion {
                x: row.get(0)?, y: row.get(1)?, width: row.get(2)?, height: row.get(3)?,
            }),
        ).optional()?)
    }

    pub fn set_pdf_region(&self, annotation: i64, region: PdfRegion) -> Result<()> {
        region.validate()?;
        let changed = self.connection.execute(
            "INSERT INTO pdf_annotation_regions(annotation_id,x,y,width,height)
             SELECT p.id,?2,?3,?4,?5 FROM pdf_annotations p
             JOIN attachments a ON a.id=p.attachment_id
             JOIN notes n ON n.id=a.note_id
             WHERE p.id=?1 AND a.kind='pdf' AND n.trash_root IS NULL
             ON CONFLICT(annotation_id) DO UPDATE
             SET x=excluded.x,y=excluded.y,width=excluded.width,height=excluded.height",
            params![annotation, region.x, region.y, region.width, region.height],
        )?;
        if changed != 1 {
            return Err("找不到 PDF 摘錄，或所屬筆記已移到垃圾桶。".into());
        }
        Ok(())
    }

    pub fn remove_pdf_region(&self, annotation: i64) -> Result<()> {
        let changed = self.connection.execute(
            "DELETE FROM pdf_annotation_regions WHERE annotation_id=?1 AND EXISTS(
                SELECT 1 FROM pdf_annotations p
                JOIN attachments a ON a.id=p.attachment_id
                JOIN notes n ON n.id=a.note_id
                WHERE p.id=?1 AND a.kind='pdf' AND n.trash_root IS NULL
             )",
            [annotation],
        )?;
        if changed != 1 {
            return Err("找不到這個範圍，或所屬筆記已移到垃圾桶。".into());
        }
        Ok(())
    }

    pub fn add_pdf_annotation_with_region(
        &self,
        attachment: i64,
        page: u32,
        quote: &str,
        comment: &str,
        region: PdfRegion,
    ) -> Result<i64> {
        self.add_pdf_mark(attachment, page, quote, comment, region, None)
    }

    #[cfg(test)]
    fn add_pdf_text_mark(
        &self, attachment: i64, page: u32, quote: &str, comment: &str, region: PdfRegion,
    ) -> Result<i64> {
        self.add_pdf_mark(attachment, page, quote, comment, region, Some(&[]))
    }

    #[cfg(any(windows, test))]
    pub fn add_pdf_text_mark_segments(
        &self, attachment: i64, page: u32, quote: &str, comment: &str, regions: &[PdfRegion],
    ) -> Result<i64> {
        let first = validate_text_mark(quote, regions)?;
        self.add_pdf_mark(attachment, page, quote, comment, first, Some(regions))
    }

    #[cfg(windows)]
    pub(crate) fn add_pdf_area_from_source(
        &self, attachment: i64, source: &[u8], page: u32, quote: &str, comment: &str, region: PdfRegion,
    ) -> Result<i64> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        verify_pdf_source(&tx, attachment, source)?;
        let id = self.write_pdf_mark(&tx, attachment, page, quote, comment, region, None)?;
        tx.commit()?;
        Ok(id)
    }

    #[cfg(any(windows, test))]
    pub(crate) fn add_pdf_text_from_source(
        &self, attachment: i64, source: &[u8], mark: PdfPageText<'_>, comment: &str,
    ) -> Result<i64> {
        let first = validate_text_mark(mark.quote, mark.regions)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        verify_pdf_source(&tx, attachment, source)?;
        let id = self.write_pdf_mark(&tx, attachment, mark.page, mark.quote, comment, first, Some(mark.regions))?;
        tx.commit()?;
        Ok(id)
    }

    #[cfg(any(windows, test))]
    pub(crate) fn add_pdf_adjacent_text(
        &self, attachment: i64, source: &[u8], first: PdfPageText<'_>, next: PdfPageText<'_>,
        comment: &str,
    ) -> Result<(i64, i64)> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let result = self.write_pdf_adjacent_text(&tx, attachment, source, first, next, comment)?;
        tx.commit()?;
        Ok(result)
    }

    #[cfg(any(windows, test))]
    pub(super) fn write_pdf_adjacent_text(
        &self, tx: &Transaction<'_>, attachment: i64, source: &[u8], first: PdfPageText<'_>,
        next: PdfPageText<'_>, comment: &str,
    ) -> Result<(i64, i64)> {
        if first.page.checked_add(1) != Some(next.page)
            || first.regions.len() + next.regions.len() > PdfRegion::MAX_TEXT_SEGMENTS
            || first.quote.encode_utf16().count() + next.quote.encode_utf16().count() >= 2048
        {
            return Err("請選取同一 PDF 的連續兩頁，合計最多 64 行與 2,048 字元。".into());
        }
        let anchors = [
            validate_text_mark(first.quote, first.regions)?,
            validate_text_mark(next.quote, next.regions)?,
        ];
        verify_pdf_source(tx, attachment, source)?;
        let first_id = self.write_pdf_mark(
            tx, attachment, first.page, first.quote, comment, anchors[0], Some(first.regions),
        )?;
        let next_id = self.write_pdf_mark(
            tx, attachment, next.page, next.quote, comment, anchors[1], Some(next.regions),
        )?;
        tx.execute("INSERT INTO pdf_text_adjacent_pairs(first_id,next_id) VALUES(?1,?2)",
            params![first_id, next_id])?;
        Ok((first_id, next_id))
    }

    #[cfg(any(windows, test))]
    pub(crate) fn pdf_adjacent_peer(&self, attachment: i64, annotation: i64) -> Result<Option<(i64, u32)>> {
        Ok(self.connection.query_row(
            "SELECT peer.id,peer.page FROM pdf_text_adjacent_pairs pair
             JOIN pdf_annotations first ON first.id=pair.first_id
             JOIN pdf_annotations next ON next.id=pair.next_id
             JOIN pdf_annotations peer ON peer.id=CASE WHEN first.id=?2 THEN next.id ELSE first.id END
             JOIN attachments a ON a.id=?1 AND a.kind='pdf'
             JOIN notes n ON n.id=a.note_id AND n.trash_root IS NULL
             WHERE (first.id=?2 OR next.id=?2) AND first.attachment_id=a.id
               AND next.attachment_id=a.id AND next.page=first.page+1",
            params![attachment, annotation], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?)
    }

    pub fn pdf_text_marks_on_page(&self, attachment: i64, page: u32) -> Result<Vec<PdfRegion>> {
        self.pdf_regions_on_page(attachment, page, 1, None)
    }

    #[cfg(any(windows, test))]
    pub fn pdf_area_regions_on_page(&self, attachment: i64, page: u32) -> Result<Vec<PdfRegion>> {
        self.pdf_regions_on_page(attachment, page, 0, None)
    }

    #[cfg(any(windows, test))]
    pub fn pdf_annotation_focus_regions(
        &self, attachment: i64, page: u32, annotation: i64,
    ) -> Result<Vec<PdfRegion>> {
        self.pdf_regions_on_page(attachment, page, 2, Some(annotation))
    }

    fn pdf_regions_on_page(
        &self, attachment: i64, page: u32, kind: i32, annotation: Option<i64>,
    ) -> Result<Vec<PdfRegion>> {
        let mut statement = self.connection.prepare(
            "SELECT coalesce(s.x,r.x),coalesce(s.y,r.y),coalesce(s.width,r.width),coalesce(s.height,r.height)
             FROM pdf_annotation_regions r
             JOIN pdf_annotations p ON p.id=r.annotation_id
             JOIN attachments a ON a.id=p.attachment_id
             JOIN notes n ON n.id=a.note_id
             LEFT JOIN pdf_text_marks m ON m.annotation_id=r.annotation_id
             LEFT JOIN pdf_text_mark_segments s ON s.annotation_id=m.annotation_id
             WHERE p.attachment_id=?1 AND p.page=?2 AND a.kind='pdf' AND n.trash_root IS NULL
                 AND ((?3=0 AND m.annotation_id IS NULL) OR (?3=1 AND m.annotation_id IS NOT NULL)
                     OR (?3=2 AND p.id=?4))
             ORDER BY p.id,s.ordinal",
        )?;
        let rows = statement.query_map(params![attachment, page, kind, annotation], |row| Ok(PdfRegion {
            x: row.get(0)?, y: row.get(1)?, width: row.get(2)?, height: row.get(3)?,
        }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn add_pdf_mark(
        &self, attachment: i64, page: u32, quote: &str, comment: &str,
        region: PdfRegion, text_segments: Option<&[PdfRegion]>,
    ) -> Result<i64> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let id = self.write_pdf_mark(&tx, attachment, page, quote, comment, region, text_segments)?;
        tx.commit()?;
        Ok(id)
    }

    fn write_pdf_mark(
        &self, tx: &Transaction<'_>, attachment: i64, page: u32, quote: &str, comment: &str,
        region: PdfRegion, text_segments: Option<&[PdfRegion]>,
    ) -> Result<i64> {
        region.validate()?;
        let id = self.add_pdf_annotation(attachment, page, quote, comment)?;
        self.set_pdf_region(id, region)?;
        if let Some(regions) = text_segments {
            tx.execute("INSERT INTO pdf_text_marks(annotation_id) VALUES(?1)", [id])?;
            for (ordinal, region) in regions.iter().enumerate() {
                tx.execute(
                    "INSERT INTO pdf_text_mark_segments(annotation_id,ordinal,x,y,width,height)
                     VALUES(?1,?2,?3,?4,?5,?6)",
                    params![id, ordinal as i64, region.x, region.y, region.width, region.height],
                )?;
            }
        }
        Ok(id)
    }
}

#[cfg(any(windows, test))]
fn validate_text_mark(quote: &str, regions: &[PdfRegion]) -> Result<PdfRegion> {
    let lines = quote.split('\n');
    let Some(first) = regions.first().copied().filter(|_| {
        regions.len() <= PdfRegion::MAX_TEXT_SEGMENTS && lines.clone().count() == regions.len()
            && lines.clone().all(|line| !line.trim().is_empty() && !line.contains('\r'))
    }) else {
        return Err("文字螢光標記須包含與選取行數相符的引文。".into());
    };
    for region in regions {
        region.validate()?;
    }
    if !text_regions_in_reading_order(regions) {
        return Err("文字反白的行範圍須依欄從左至右，欄內由上而下排列，且不可重疊。".into());
    }
    Ok(first)
}

#[cfg(any(windows, test))]
fn verify_pdf_source(tx: &Transaction<'_>, attachment: i64, content: &[u8]) -> Result<()> {
    let same: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM attachments a JOIN notes n ON n.id=a.note_id
         WHERE a.id=?1 AND a.kind='pdf' AND a.content=?2 AND n.trash_root IS NULL)",
        params![attachment, content], |row| row.get(0),
    )?;
    if !same {
        return Err("PDF 原始位元已變更，或來源目前無法使用。請重新開啟原檔再選取。".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const REGION: PdfRegion = PdfRegion { x: 0.25, y: 0.5, width: 0.75, height: 0.25 };

    #[test]
    fn page_regions_require_finite_positive_in_bounds_coordinates() {
        REGION.validate().unwrap();
        PdfRegion { x: 0.8, width: 0.2, ..REGION }.validate().unwrap();
        for invalid in [
            PdfRegion { x: -0.01, ..REGION },
            PdfRegion { x: 1.0, ..REGION },
            PdfRegion { y: f64::NAN, ..REGION },
            PdfRegion { width: 0.0, ..REGION },
            PdfRegion { width: f64::INFINITY, ..REGION },
            PdfRegion { width: 0.8, ..REGION },
            PdfRegion { height: 0.6, ..REGION },
        ] {
            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    fn two_column_text_regions_reopen_in_reading_order_and_reject_other_wraps() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("notes.sqlite3");
        let source = dir.join("兩欄.pdf");
        let backup = dir.join("backup.sqlite3");
        fs::write(&source, b"%PDF-2.0\n")?;
        let mut db = Notebook::open(&path, false)?;
        let note = db.add(None, "雙欄來源", "")?;
        let attachment = db.import_attachment(note, &source)?;
        let first = PdfRegion { x: 0.1, y: 0.2, width: 0.2, height: 0.03 };
        let second = PdfRegion { y: 0.3, ..first };
        let third = PdfRegion { x: 0.6, y: 0.1, ..first };
        let fourth = PdfRegion { y: 0.2, ..third };
        let regions = [first, second, third, fourth];
        let quote = "左欄第二行\n左欄第三行\n右欄第一行\n右欄第二行";
        for invalid in [
            [first, third, second, fourth],
            [third, fourth, first, second],
            [first, second, PdfRegion { x: 0.15, ..third }, fourth],
        ] {
            assert!(db.add_pdf_text_mark_segments(attachment, 2, quote, "", &invalid).is_err());
        }
        assert!(db.list_pdf_annotations(attachment)?.is_empty());
        let id = db.add_pdf_text_mark_segments(attachment, 2, quote, "兩欄順序", &regions)?;
        db.backup(&backup)?;
        drop(db);

        let reopened = Notebook::open(&backup, false)?;
        let marks = reopened.list_pdf_annotations(attachment)?;
        assert_eq!((marks[0].id, marks[0].quote.as_str()), (id, quote));
        assert_eq!(reopened.pdf_annotation_focus_regions(attachment, 2, id)?, regions);
        assert_eq!(reopened.pdf_text_marks_on_page(attachment, 2)?, regions);
        drop(reopened);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn consecutive_pdf_pages_are_atomic_and_reopen_as_two_exact_anchors() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let source_file = dir.join("雙頁.pdf");
        let source = b"%PDF-1.7\noriginal";
        fs::write(&source_file, source)?;
        let mut db = Notebook::open(&dir.join("notes.sqlite3"), false)?;
        let owner = db.add(None, "雙頁來源", "")?;
        let attachment = db.import_attachment(owner, &source_file)?;
        let other = db.import_attachment(owner, &source_file)?;
        let first = [PdfRegion { x: 0.12, y: 0.75, width: 0.45, height: 0.025 }];
        let next = [PdfRegion { x: 0.14, y: 0.04, width: 0.26, height: 0.027 }];
        let start = PdfPageText { page: 1, quote: "第一頁原文", regions: &first };
        let end = PdfPageText { page: 2, quote: "下一頁原文", regions: &next };
        assert!(db.add_pdf_adjacent_text(attachment, source, start, PdfPageText {page: 3, ..end}, "").is_err());
        db.connection.execute("UPDATE attachments SET content=?1 WHERE id=?2",
            params![b"%PDF-1.7\nchanged".as_slice(), attachment])?;
        assert!(db.add_pdf_adjacent_text(attachment, source, start, end, "").is_err());
        db.connection.execute("UPDATE attachments SET content=?1 WHERE id=?2", params![source.as_slice(), attachment])?;

        db.connection.execute_batch(
            "CREATE TEMP TRIGGER reject_next_page BEFORE INSERT ON pdf_annotations WHEN NEW.page=2
             BEGIN SELECT RAISE(ABORT,'synthetic next page failure'); END;"
        )?;
        assert!(db.add_pdf_adjacent_text(attachment, source, start, end, "").is_err());
        assert!(db.list_pdf_annotations(attachment)?.is_empty());
        db.connection.execute_batch("DROP TRIGGER reject_next_page;")?;
        let (first_id, next_id) = db.add_pdf_adjacent_text(attachment, source, start, end, "相鄰兩頁")?;
        assert_eq!(db.pdf_adjacent_peer(attachment, first_id)?, Some((next_id, 2)));
        assert_eq!(db.pdf_adjacent_peer(other, first_id)?, None);
        let backup = dir.join("backup.sqlite3");
        db.backup(&backup)?;
        drop(db);

        let db = Notebook::open(&backup, false)?;
        assert_eq!(db.pdf_adjacent_peer(attachment, next_id)?, Some((first_id, 1)));
        assert_eq!(db.pdf_annotation_focus_regions(attachment, 1, first_id)?, first);
        assert_eq!(db.pdf_annotation_focus_regions(attachment, 2, next_id)?, next);
        let annotations = db.list_pdf_annotations(attachment)?;
        assert_eq!((annotations[0].quote.as_str(), annotations[1].quote.as_str()), (start.quote, end.quote));
        assert!(annotations.iter().all(|entry| entry.comment == "相鄰兩頁"));
        db.remove_pdf_annotation(first_id)?;
        assert_eq!(db.pdf_adjacent_peer(attachment, next_id)?, None);
        assert_eq!(db.pdf_annotation_focus_regions(attachment, 2, next_id)?, next);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn upgrades_v9_marks_and_keeps_multiline_segments_atomic_and_distinct() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-pdf-lines-{}", std::process::id()));
        fs::create_dir(&dir)?;
        let path = dir.join("notes.sqlite3");
        let source = dir.join("fixture.pdf");
        let backup = dir.join("backup.sqlite3");
        fs::write(&source, b"%PDF-2.0\n")?;
        let (attachment, legacy, area) = {
            let mut db = Notebook::open(&path, false)?;
            let note = db.add(None, "跨行文字", "")?;
            let attachment = db.import_attachment(note, &source)?;
            let legacy = db.add_pdf_text_mark(attachment, 2, "舊引文", "", REGION)?;
            let area = db.add_pdf_annotation_with_region(attachment, 2, "", "選區", REGION)?;
            crate::store::tests::remove_v11_schema(&db.connection)?;
            db.connection.execute_batch(
                "DROP TABLE pdf_text_mark_segments; DROP TABLE vault_push_verified; PRAGMA user_version=9;",
            )?;
            (attachment, legacy, area)
        };
        let first = PdfRegion { x: 0.12, y: 0.16, width: 0.34, height: 0.04 };
        let second = PdfRegion { x: 0.07, y: 0.28, width: 0.27, height: 0.05 };
        let quote = "第一行引文\nSecond line";
        let id = {
            let db = Notebook::open(&path, false)?;
            assert_eq!(db.pdf_text_marks_on_page(attachment, 2)?, [REGION]);
            assert_eq!(db.pdf_annotation_focus_regions(attachment, 2, legacy)?, [REGION]);
            assert_eq!(db.pdf_area_regions_on_page(attachment, 2)?, [REGION]);
            assert_eq!(db.pdf_annotation_focus_regions(attachment, 2, area)?, [REGION]);
            for bad in [&[][..], &[first][..], &[second, first][..], &[first, first][..]] {
                assert!(db.add_pdf_text_mark_segments(attachment, 2, quote, "", bad).is_err());
            }
            db.connection.execute_batch(
                "CREATE TEMP TRIGGER fail_second_pdf_line BEFORE INSERT ON pdf_text_mark_segments
                 WHEN new.ordinal=1 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
            )?;
            assert!(db.add_pdf_text_mark_segments(attachment, 2, quote, "", &[first, second]).is_err());
            assert_eq!(db.list_pdf_annotations(attachment)?.len(), 2);
            assert_eq!(db.pdf_text_marks_on_page(attachment, 2)?, [REGION]);
            db.connection.execute_batch("DROP TRIGGER fail_second_pdf_line")?;
            let id = db.add_pdf_text_mark_segments(attachment, 2, quote, "重開後比對", &[first, second])?;
            db.backup(&backup)?;
            id
        };
        {
            let restored = Notebook::open(&backup, false)?;
            let annotations = restored.list_pdf_annotations(attachment)?;
            assert_eq!((annotations[2].id, annotations[2].quote.as_str()), (id, quote));
            assert_eq!(restored.pdf_region(id)?, Some(first));
            assert_eq!(restored.pdf_annotation_focus_regions(attachment, 2, id)?, [first, second]);
            assert!(restored.pdf_annotation_focus_regions(attachment, 3, id)?.is_empty());
            assert_eq!(restored.pdf_text_marks_on_page(attachment, 2)?, [REGION, first, second]);
            restored.remove_pdf_annotation(id)?;
            assert_eq!(restored.pdf_text_marks_on_page(attachment, 2)?, [REGION]);
            let segments: i64 = restored.connection.query_row(
                "SELECT count(*) FROM pdf_text_mark_segments", [], |row| row.get(0),
            )?;
            assert_eq!(segments, 0);
        }
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn region_retains_its_page_survives_backup_and_tracks_attachment_lifecycle() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("paperroom-regions-{}", std::process::id()));
        fs::create_dir(&dir)?;
        let db_path = dir.join("notes.sqlite3");
        let pdf_path = dir.join("fixture.pdf");
        let backup = dir.join("backup.sqlite3");
        fs::write(&pdf_path, b"%PDF-2.0\n")?;
        {
            let mut first = Notebook::open(&db_path, false)?;
            let second = Notebook::open(&db_path, false)?;
            let note = first.add(None, "範圍測試", "")?;
            let attachment = first.import_attachment(note, &pdf_path)?;
            let wrong = PdfRegion { height: 1.0, ..REGION };
            assert!(first.add_pdf_annotation_with_region(attachment, 9, "", "批註", wrong).is_err());
            assert!(first.list_pdf_annotations(attachment)?.is_empty());
            first.connection.execute_batch(
                "CREATE TEMP TRIGGER region_test_failure BEFORE INSERT ON pdf_annotation_regions
                 BEGIN SELECT RAISE(ABORT, 'synthetic region write failure'); END;",
            )?;
            assert!(first.add_pdf_annotation_with_region(attachment, 9, "", "批註", REGION).is_err());
            assert!(second.list_pdf_annotations(attachment)?.is_empty());
            first.connection.execute_batch("DROP TRIGGER region_test_failure;")?;
            let text = first.add_pdf_annotation(attachment, 3, "原文", "一般摘錄")?;
            let anchored = first.add_pdf_annotation_with_region(attachment, 9, "", "有範圍", REGION)?;
            assert_eq!(first.pdf_region(text)?, None);
            let mark = first.add_pdf_text_mark(attachment, 9, "可見引文", "", REGION)?;
            assert_ne!(mark, anchored);
            assert_eq!(first.pdf_text_marks_on_page(attachment, 9)?, [REGION]);
            assert!(first.pdf_text_marks_on_page(attachment, 3)?.is_empty());
            assert_eq!(first.pdf_region(anchored)?, Some(REGION));
            first.set_pdf_region(text, REGION)?;
            first.remove_pdf_region(text)?;
            assert_eq!(first.pdf_region(text)?, None);
            first.trash(note)?;
            assert_eq!(second.pdf_region(anchored)?, None);
            assert!(second.set_pdf_region(anchored, REGION).is_err());
            assert!(second.remove_pdf_region(anchored).is_err());
            first.restore(note)?;
            first.backup(&backup)?;
            let restored = Notebook::open(&backup, false)?;
            let annotations = restored.list_pdf_annotations(attachment)?;
            assert_eq!((annotations[1].id, annotations[1].page), (anchored, 9));
            assert_eq!(restored.pdf_region(anchored)?, Some(REGION));
            assert_eq!(restored.pdf_text_marks_on_page(attachment, 9)?, [REGION]);
            first.remove_attachment(attachment)?;
            assert!(first.pdf_text_marks_on_page(attachment, 9)?.is_empty());
            assert!(first.set_pdf_region(anchored, REGION).is_err());
            let remaining: i64 = first.connection.query_row(
                "SELECT count(*) FROM pdf_annotation_regions", [], |row| row.get(0),
            )?;
            assert_eq!(remaining, 0);
        }
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
