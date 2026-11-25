use super::{Notebook, Result, collection_views::{CalendarDay, Month}, tasks::task_markers};
use rusqlite::params;

const MAX_MONTH_TASK_SCAN: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalDay {
    Active { pending: Option<usize> },
    Trashed,
}

impl Notebook {
    pub(crate) fn journal_month(&self, month: Month) -> Result<[Option<JournalDay>; 31]> {
        if Month::parse(&month.iso()) != Some(month) {
            return Err("無效的日誌月份。".into());
        }
        let (first, last) = month.bounds();
        let mut query = self.connection.prepare_cached(
            "SELECT d.day,n.trash_root IS NOT NULL,
                    CASE WHEN n.trash_root IS NULL AND instr(n.body,'[')>0 THEN n.body END
             FROM daily_notes d JOIN notes n ON n.id=d.note_id
             WHERE d.day BETWEEN ?1 AND ?2 ORDER BY d.day",
        )?;
        let mut rows = query.query(params![first.iso(), last.iso()])?;
        let mut days = [None; 31];
        let mut scan_budget = MAX_MONTH_TASK_SCAN;
        while let Some(row) = rows.next()? {
            let day = CalendarDay::parse(row.get_ref(0)?.as_str()?)
                .filter(|day| day.month() == month).ok_or("工作區含有無效的日誌日期。")?;
            let state = if row.get(1)? {
                JournalDay::Trashed
            } else if let Some(body) = row.get_ref(2)?.as_str_or_null()? {
                let pending = scan_budget.checked_sub(body.len()).map(|remaining| {
                    scan_budget = remaining;
                    task_markers(body).filter(|(_, done)| !*done).count()
                });
                JournalDay::Active { pending }
            } else {
                JournalDay::Active { pending: Some(0) }
            };
            days[day.day() - 1] = Some(state);
        }
        Ok(days)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_calendar_uses_saved_date_identity_and_survives_backup() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("journal.sqlite3");
        let backup = dir.join("restored.sqlite3");
        let month = Month::parse("2024-02").unwrap();
        let mut db = Notebook::open(&path, false)?;
        assert_eq!(db.journal_month(month)?, [None; 31]);
        let leap = db.open_daily(2024, 2, 29)?;
        db.save(leap, 1, "改名後的閏日日誌",
            "- [ ] 訪問\n- [x] 整理\n\n```text\n- [ ] 只是範例\n```\n")?;
        db.add(None, "2024-02-28", "- [ ] 筆記標題和日期相同")?;
        let march = db.open_daily(2024, 3, 1)?;
        assert_eq!(db.journal_month(month)?[27], None);
        assert_eq!(db.journal_month(month)?[28], Some(JournalDay::Active { pending: Some(1) }));
        assert_eq!(db.journal_month(month)?[29], None);

        let mut other = Notebook::open(&path, false)?;
        assert_eq!(other.open_daily(2024, 2, 29)?, leap);
        db.trash(leap)?;
        assert_eq!(other.journal_month(month)?[28], Some(JournalDay::Trashed));
        assert!(other.open_daily(2024, 2, 29).is_err());
        db.restore(leap)?;
        assert_eq!(other.open_daily(2024, 2, 29)?, leap);
        assert_eq!(other.journal_month(month.shifted(1))?[0], Some(JournalDay::Active { pending: Some(0) }));
        assert_eq!(other.open_daily(2024, 3, 1)?, march);
        drop(other);

        db.backup(&backup)?;
        drop(db);
        let mut restored = Notebook::open(&backup, false)?;
        assert_eq!(restored.journal_month(month)?[28], Some(JournalDay::Active { pending: Some(1) }));
        assert_eq!(restored.open_daily(2024, 2, 29)?, leap);
        assert_eq!(restored.note(leap)?.unwrap().row.title, "改名後的閏日日誌");
        let new = restored.open_daily(2024, 2, 28)?;
        assert_ne!(new, leap);
        assert_eq!(restored.open_daily(2024, 2, 28)?, new);
        assert_eq!(restored.journal_month(month)?[27], Some(JournalDay::Active { pending: Some(0) }));
        drop(restored);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
