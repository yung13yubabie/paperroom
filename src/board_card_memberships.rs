//! Navigation joins a native placement to ordinary collection rows by canonical note ID.
use super::{NativeBoardLocation, Notebook, Result};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CardMembership {
    pub(crate) collection: i64,
    pub(crate) title: String,
    status: String,
    note: i64,
}

impl CardMembership {
    pub(crate) fn label(&self) -> String {
        format!("{}　[#{}]　｜ {}", self.title, self.collection, self.status)
    }
}

impl Notebook {
    // Caller holds the read transaction shared with placement and layout validation.
    fn memberships_for_card(&self, origin: &NativeBoardLocation) -> Result<Vec<CardMembership>> {
        self.validate_board_path(origin)?;
        self.active_placement_name(origin)?;
        let mut stmt = self.connection.prepare_cached(
            "SELECT c.id,c.name,m.status FROM collection_notes m JOIN collections c ON c.id=m.collection_id
             WHERE m.note_id=?1 AND NOT EXISTS(SELECT 1 FROM obsidian_base_dynamic d WHERE d.collection_id=c.id)
             ORDER BY c.name COLLATE BINARY,c.id",
        )?;
        Ok(stmt.query_map([origin.note], |row| Ok(CardMembership {
            collection: row.get(0)?, title: row.get(1)?, status: row.get(2)?, note: origin.note,
        }))?.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn card_memberships(&self, origin: &NativeBoardLocation) -> Result<Vec<CardMembership>> {
        let tx = self.connection.unchecked_transaction()?;
        let rows = self.memberships_for_card(origin)?;
        tx.commit()?;
        Ok(rows)
    }

    pub(crate) fn read_card_membership<T>(&self, origin: &NativeBoardLocation, selected: &CardMembership,
        read_layout: impl FnOnce() -> Result<T>) -> Result<T> {
        let tx = self.connection.unchecked_transaction()?;
        if !self.memberships_for_card(origin)?.contains(selected) {
            return Err("集合成員或狀態已變更，請重新選取。".into());
        }
        let layout = read_layout()?;
        tx.commit()?;
        Ok(layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn card_membership_requires_original_placement_and_same_row_after_reopen_and_backup() -> Result<()> {
        let dir = crate::store::tests::data_dir();
        let path = dir.join("card.sqlite3");
        let backup = dir.join("copy.sqlite3");
        let mut db = Notebook::open(&path,false)?;
        let real = db.add(None,"同名","正文甲")?;
        let decoy = db.add(None,"同名","正文乙")?;
        let board = db.create_board("素材")?;
        db.add_card_on(board,real)?;
        db.add_card_on(board,decoy)?;
        let a = db.create_collection("A")?;
        let b = db.create_collection("B")?;
        db.add_to_collection(a,decoy,"待辦")?;
        db.add_to_collection(a,real,"已選")?;
        db.add_to_collection(b,real,"已完成")?;
        let origin = db.native_card_origin(board,real)?;
        let decoy_origin = db.native_card_origin(board,decoy)?;
        let memberships = db.card_memberships(&origin)?;
        assert_eq!(memberships.iter().map(|m|m.collection).collect::<Vec<_>>(),[a,b]);
        assert!(db.read_card_membership(&decoy_origin,&memberships[0],||Ok(())).is_err());
        let rows = db.read_card_membership(&origin,&memberships[0],||db.collection_rows(a))?;
        assert!(rows.iter().any(|(row,status)|row.id==real && status=="已選"));
        assert!(rows.iter().any(|(row,status)|row.id==decoy && status=="待辦"));
        db.backup(&backup)?;
        drop(db);
        let mut db = Notebook::open(&path,false)?;
        assert_eq!(db.card_memberships(&origin)?,memberships);
        db.move_collection_note(a,real,"已選","處理中")?;
        assert!(db.read_card_membership(&origin,&memberships[0],||Ok(())).is_err());
        db.trash(real)?;
        assert!(db.card_memberships(&origin).is_err());
        db.restore(real)?;
        assert_eq!(db.card_memberships(&origin)?.len(),2);
        db.remove_card_on(board,real)?;
        assert!(db.card_memberships(&origin).is_err());
        let restored = Notebook::open(&backup,false)?;
        assert_eq!(restored.card_memberships(&origin)?,memberships);
        assert_eq!(restored.note(real)?.unwrap().body,"正文甲");
        drop(restored);
        drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
