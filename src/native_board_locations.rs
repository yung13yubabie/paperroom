//! Reverse lookup of native board placements by canonical note identity.
#[path = "board_card_memberships.rs"]
mod board_card_memberships;
use super::{Note, Notebook, Result, read_row};
use rusqlite::{OptionalExtension, params};

#[derive(Clone)]
pub(crate) struct NativeBoardLocation {
    pub board: i64,
    pub title: String,
    pub note: i64,
    token: String,
    ancestors: Vec<(i64,String)>,
}

pub(crate) struct EdgeEndpoint {
    pub(crate) location: NativeBoardLocation,
    name: String,
}

impl EdgeEndpoint {
    pub(crate) fn label(&self) -> String { format!("[#{}]　{}",self.location.note,self.name) }
}

impl NativeBoardLocation {
    pub(crate) fn breadcrumb(&self) -> String {
        self.ancestors.iter().map(|(_,title)|title.as_str()).collect::<Vec<_>>().join(" › ")
    }
}

pub(crate) fn location_labels(locations:&[NativeBoardLocation]) -> Vec<String> {
    let paths = locations.iter().map(NativeBoardLocation::breadcrumb).collect::<Vec<_>>();
    let mut counts = std::collections::HashMap::new();
    for path in &paths { *counts.entry(path.as_str()).or_insert(0) += 1; }
    let ambiguous = paths.iter().any(|path| counts[path.as_str()] > 1);
    paths.into_iter().zip(locations).map(|(path,location)| {
        if ambiguous { format!("{path}　[#{}]",location.board) } else { path }
    }).collect()
}

impl Notebook {
    fn board_location_path(&self, board:i64) -> Result<Vec<(i64,String)>> {
        Ok(self.board_path(board)?.into_iter().map(|b|(b.id,b.title)).collect())
    }

    fn location_rows(&self, note:i64, only:Option<i64>) -> Result<Vec<NativeBoardLocation>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT b.id,b.title,i.token FROM boards b
             JOIN board_cards c ON c.board_id=b.id
             JOIN notes n ON n.id=c.note_id JOIN note_link_identities i ON i.note_id=n.id
             WHERE n.id=?1 AND n.trash_root IS NULL AND (?2 IS NULL OR b.id=?2) ORDER BY b.id",
        )?;
        let mut locations = Vec::new();
        for row in statement.query_map(params![note,only],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))? {
            let (board,title,token) = row?;
            locations.push(NativeBoardLocation {board,title,note,token,ancestors:self.board_location_path(board)?});
        }
        Ok(locations)
    }

    pub(crate) fn native_board_locations(&self, note: i64) -> Result<Vec<NativeBoardLocation>> {
        let tx = self.connection.unchecked_transaction()?;
        let rows = self.location_rows(note,None)?;
        tx.commit()?;
        Ok(rows)
    }

    pub(crate) fn native_card_origin(&self, board:i64, note:i64) -> Result<NativeBoardLocation> {
        let tx = self.connection.unchecked_transaction()?;
        let row = self.location_rows(note,Some(board))?.pop().ok_or("來源卡片已離開白板。")?;
        tx.commit()?;
        Ok(row)
    }

    pub(crate) fn other_card_locations(&self, origin:&NativeBoardLocation) -> Result<Vec<NativeBoardLocation>> {
        let tx = self.connection.unchecked_transaction()?;
        self.validate_board_path(origin)?;
        let rows = self.location_rows(origin.note,None)?;
        if !rows.iter().any(|r|r.board==origin.board && r.token==origin.token) { return Err("來源卡片已變更。".into()); }
        tx.commit()?;
        Ok(rows.into_iter().filter(|r|r.board!=origin.board).collect())
    }

    fn active_placement_name(&self, location: &NativeBoardLocation) -> Result<String> {
        Ok(self.connection.query_row("SELECT n.title FROM board_cards c
            JOIN notes n ON n.id=c.note_id JOIN note_link_identities i ON i.note_id=n.id
            WHERE c.board_id=?1 AND n.id=?2 AND i.token=?3 AND n.trash_root IS NULL",
            params![location.board,location.note,location.token],|r|r.get(0))
            .optional()?.ok_or("卡片已離開白板或身分已變更。")?)
    }

    pub(crate) fn edge_endpoints(&self, origin:&NativeBoardLocation, connected:bool) -> Result<Vec<EdgeEndpoint>> {
        let tx=self.connection.unchecked_transaction()?;
        self.validate_board_path(origin)?;
        self.active_placement_name(origin)?;
        let mut stmt=tx.prepare("SELECT n.id,n.title,i.token FROM board_cards c JOIN notes n ON c.note_id=n.id
            JOIN note_link_identities i ON i.note_id=n.id WHERE c.board_id=?1 AND n.id<>?2 AND n.trash_root IS NULL
            AND EXISTS(SELECT 1 FROM board_edges e WHERE e.board_id=c.board_id
                AND e.from_note=min(n.id,?2) AND e.to_note=max(n.id,?2))=?3 ORDER BY n.id")?;
        let endpoints=stmt.query_map(params![origin.board,origin.note,connected],|r|Ok(EdgeEndpoint {
            location:NativeBoardLocation { note:r.get(0)?, token:r.get(2)?, ..origin.clone() }, name:r.get(1)?,
        }))?.collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        tx.commit()?;
        Ok(endpoints)
    }

    fn validate_edge_endpoint(&self, origin:&NativeBoardLocation, endpoint:&EdgeEndpoint, connected:bool) -> Result<()> {
        let target=&endpoint.location;
        if origin.board!=target.board || origin.ancestors!=target.ancestors || origin.note==target.note {
            return Err("請重新選取目前白板的連線端點。".into());
        }
        self.validate_board_path(origin)?;
        self.active_placement_name(origin)?;
        if self.active_placement_name(target)?!=endpoint.name { return Err("目的卡片名稱已變更，請重新選取。".into()); }
        let exists:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM board_edges WHERE board_id=?1
            AND from_note=min(?2,?3) AND to_note=max(?2,?3))",params![origin.board,origin.note,target.note],|r|r.get(0))?;
        if exists!=connected { return Err("兩張卡片的連線已變更，請重新選取。".into()); }
        Ok(())
    }

    pub(crate) fn follow_card_edge(&self, origin:&NativeBoardLocation, endpoint:&EdgeEndpoint) -> Result<()> {
        let tx=self.connection.unchecked_transaction()?;
        self.validate_edge_endpoint(origin,endpoint,true)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn edit_card_edge(&self, origin:&NativeBoardLocation, endpoint:&EdgeEndpoint, add:bool) -> Result<()> {
        let tx=rusqlite::Transaction::new_unchecked(&self.connection,rusqlite::TransactionBehavior::Immediate)?;
        self.validate_edge_endpoint(origin,endpoint,!add)?;
        if add { self.add_edge_on(origin.board,origin.note,endpoint.location.note)?; }
        else { self.remove_edge_on(origin.board,origin.note,endpoint.location.note)?; }
        tx.commit()?;
        Ok(())
    }

    fn validate_board_path(&self, location:&NativeBoardLocation) -> Result<()> {
        if self.board_location_path(location.board)? != location.ancestors {
            return Err("白板路徑已變更，請重新查看白板位置。".into());
        }
        Ok(())
    }

    /// Both ends of a card-to-card jump must still belong to the displayed note and paths.
    pub(crate) fn check_card_jump(&self, origin:&NativeBoardLocation, target:&NativeBoardLocation) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        if origin.note!=target.note || origin.token!=target.token || origin.board==target.board {
            return Err("卡片位置不相符。".into());
        }
        self.checked_location_note(origin)?;
        self.checked_location_note(target)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn note_at_native_board_location(&self, location: &NativeBoardLocation) -> Result<Note> {
        let tx = self.connection.unchecked_transaction()?;
        let note = self.checked_location_note(location)?;
        tx.commit()?;
        Ok(note)
    }

    fn checked_location_note(&self, location: &NativeBoardLocation) -> Result<Note> {
        self.validate_board_path(location)?;
        self.connection.query_row(
            "SELECT n.id,n.parent,n.title,n.pinned,n.trash_root,n.body,n.version
             FROM board_cards c JOIN boards b ON b.id=c.board_id
             JOIN notes n ON n.id=c.note_id JOIN note_link_identities i ON i.note_id=n.id
             WHERE b.id=?1 AND b.title=?2 AND n.id=?3 AND i.token=?4 AND n.trash_root IS NULL",
            params![location.board, location.title, location.note, location.token],
            |row| Ok(Note { row: read_row(row)?, body: row.get(5)?, version: row.get(6)? }),
        ).optional()?.ok_or_else(|| "白板位置已移除，或筆記已無法開啟。請重新查看白板位置。".into())
    }
}

pub(crate) fn reveal_offset(position: i32, size: i32, viewport: i32) -> i32 {
    if position + size <= viewport { return 0; }
    (position - (viewport - size).max(0) / 2).max(0)
}

/// Allow centering ordinary cards and panning to the trailing edge of oversized cards.
pub(crate) fn pan_limit(position: i32, size: i32, viewport: i32) -> i32 {
    position + (size - viewport).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{collections::RelationMember, named_relation_navigation::Direction};
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};

    #[test]
    fn card_jump_rechecks_both_branches_and_distinguishes_same_title_notes() -> Result<()> {
        let root=super::super::tests::data_dir();
        let path=root.join("live.db");
        let backup=root.join("backup.db");
        let mut db=Notebook::open(&path,false)?;
        let a=db.add(None,"鏡頭","原卡")?;
        let b=db.add(None,"鏡頭","同名另一卡")?;
        let top=db.create_board("研究")?;
        let source=db.new_child_board(top,"收件")?;
        let nested=db.new_child_board(top,"場記")?;
        let leaf=db.new_child_board(nested,"訪談")?;
        let lookalike=db.create_board("研究 › 場記 › 訪談")?;
        db.add_card_on(source,a)?;
        db.add_card_on(leaf,a)?;
        db.add_card_on(leaf,b)?;
        db.add_card_on(lookalike,a)?;
        db.move_card_on(leaf,a,3250,780)?;
        let origin=db.native_card_origin(source,a)?;
        let destinations=db.other_card_locations(&origin)?;
        assert_eq!(destinations.iter().map(|p|p.board).collect::<Vec<_>>(),[leaf,lookalike]);
        assert_eq!(location_labels(&destinations),[format!("研究 › 場記 › 訪談　[#{leaf}]"),format!("研究 › 場記 › 訪談　[#{lookalike}]")]);
        assert!(db.other_card_locations(&db.native_card_origin(leaf,b)?)?.is_empty());
        assert!(db.check_card_jump(&origin,&db.native_card_origin(leaf,b)?).is_err());
        db.check_card_jump(&origin,&destinations[0])?;
        db.backup(&backup)?;
        let mut writer=Notebook::open(&path,false)?;
        writer.reparent_board(nested,Some(top),None)?;
        assert!(db.check_card_jump(&origin,&destinations[0]).is_err());
        let refreshed=db.other_card_locations(&origin)?;
        assert_eq!(refreshed[0].breadcrumb(),"場記 › 訪談");
        writer.reparent_board(source,Some(top),None)?;
        assert!(db.check_card_jump(&origin,&refreshed[0]).is_err());
        assert!(db.other_card_locations(&origin).is_err());
        let new_origin=db.native_card_origin(source,a)?;
        writer.remove_card_on(source,a)?;
        assert!(db.check_card_jump(&new_origin,&refreshed[0]).is_err());
        drop(writer);drop(db);
        let restored=Notebook::open(&backup,false)?;
        restored.check_card_jump(&origin,&destinations[0])?;
        let located=restored.note_at_native_board_location(&destinations[0])?;
        assert_eq!((located.row.id,located.body.as_str()),(a,"原卡"));
        assert!(restored.cards_on(leaf)?.iter().any(|card|card.id==a && (card.x,card.y)==(3250,780)));
        drop(restored);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn edge_picker_follows_only_the_chosen_canonical_neighbor_and_rechecks_edits() -> Result<()> {
        let dir=super::super::tests::data_dir();
        let path=dir.join("live.db");
        let backup=dir.join("backup.db");
        let mut db=Notebook::open(&path,false)?;
        let target=db.add(None,"鏡頭","正本")?;
        let decoy=db.add(None,"鏡頭","備選")?;
        let a=db.add(None,"場記","來源")?;
        let root=db.create_board("組織")?;
        let board=db.new_child_board(root,"工作板")?;
        let elsewhere=db.create_board("另一板")?;
        for note in [a,target,decoy] { db.add_card_on(board,note)?; }
        for note in [a,decoy] { db.add_card_on(elsewhere,note)?; }
        db.move_card_on(board,target,3400,3000)?;
        db.add_edge_on(elsewhere,a,decoy)?;
        db.set_relation(a,"參考",decoy,true)?;
        let bytes=dir.join("場記.txt");
        fs::write(&bytes,"正本附件")?;
        let file=db.import_attachment(target,&bytes)?;
        let origin=db.native_card_origin(board,a)?;
        let choices=db.edge_endpoints(&origin,false)?;
        assert_eq!(choices.iter().map(EdgeEndpoint::label).collect::<Vec<_>>(),
            [format!("[#{target}]　鏡頭"),format!("[#{decoy}]　鏡頭")]);
        db.edit_card_edge(&origin,&choices[0],true)?;
        assert!(db.edit_card_edge(&origin,&choices[0],true).is_err());
        let linked=db.edge_endpoints(&origin,true)?;
        assert_eq!(linked.len(),1);
        let selected=&linked[0];
        let observed:i64=db.connection.query_row("SELECT total_changes()",[],|r|r.get(0))?;
        db.follow_card_edge(&origin,selected)?;
        assert_eq!(db.connection.query_row("SELECT total_changes()",[],|r|r.get::<_,i64>(0))?,observed);
        let opposite=db.native_card_origin(board,target)?;
        assert_eq!(db.edge_endpoints(&opposite,true)?[0].location.note,a);
        let mut writer=Notebook::open(&path,false)?;
        writer.remove_edge_on(board,a,target)?;
        assert!(db.follow_card_edge(&origin,selected).is_err());
        assert!(db.edit_card_edge(&origin,selected,false).is_err());
        writer.add_edge_on(board,a,target)?;
        writer.save(target,1,"新鏡頭","正本")?;
        assert!(db.follow_card_edge(&origin,selected).is_err());
        let current=db.edge_endpoints(&origin,true)?.remove(0);
        db.follow_card_edge(&origin,&current)?;
        db.backup(&backup)?;
        writer.reparent_board(board,Some(root),None)?;
        assert!(db.edit_card_edge(&origin,&current,false).is_err());
        let refreshed=db.native_card_origin(board,a)?;
        let next=db.edge_endpoints(&refreshed,true)?.remove(0);
        writer.trash(target)?;
        assert!(db.follow_card_edge(&refreshed,&next).is_err());
        writer.restore(target)?;
        db.edit_card_edge(&refreshed,&next,false)?;
        assert!(db.edges_on(board)?.is_empty());
        assert_eq!(db.edges_on(elsewhere)?.len(),1);
        drop(writer);drop(db);
        let db=Notebook::open(&backup,false)?;
        db.follow_card_edge(&origin,&current)?;
        assert_eq!(db.note_at_native_board_location(&current.location)?.body,"正本");
        assert_eq!(db.relations(a)?[0].1.id,decoy);
        assert!(db.cards_on(board)?.iter().any(|v|v.id==target && (v.x,v.y)==(3400,3000)));
        db.save_attachment_copy(file,&dir.join("匯出.txt"))?;
        assert_eq!(fs::read(dir.join("匯出.txt"))?,"正本附件".as_bytes());
        drop(db);fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn reusing_a_deleted_numeric_id_cannot_retarget_the_stale_edge_picker() -> Result<()> {
        let dir=super::super::tests::data_dir();
        let file=dir.join("reuse.db");
        let mut db=Notebook::open(&file,false)?;
        let from=db.add(None,"共同","來源")?;
        let old=db.add(None,"同一個標題","過去的正本")?;
        db.add_card_on(1,from)?;
        db.add_card_on(1,old)?;
        db.add_edge_on(1,from,old)?;
        let origin=db.native_card_origin(1,from)?;
        let old_target=db.edge_endpoints(&origin,true)?.remove(0);
        let mut writer=Notebook::open(&file,false)?;
        writer.remove_card_on(1,old)?;
        // Represent a permanently discarded note in a synthetic database.
        writer.connection.execute("DELETE FROM notes WHERE id=?1",[old])?;
        let new=writer.add(None,"同一個標題","另一篇新的正文")?;
        assert_eq!(new,old);
        writer.add_card_on(1,new)?;
        writer.add_edge_on(1,from,new)?;
        assert!(db.follow_card_edge(&origin,&old_target).is_err());
        assert!(db.edit_card_edge(&origin,&old_target,false).is_err());
        let fresh=db.edge_endpoints(&origin,true)?.remove(0);
        db.follow_card_edge(&origin,&fresh)?;
        assert_eq!(db.note_at_native_board_location(&fresh.location)?.body,"另一篇新的正文");
        drop(writer);drop(db);fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn viewport_offsets_reach_oversized_items_and_center_distant_cards() {
        assert_eq!(reveal_offset(16, 232, 500), 0);
        assert_eq!(reveal_offset(4216, 232, 500), 4082);
        assert_eq!(reveal_offset(16, 768, 500), 16);
        assert_eq!(pan_limit(16, 768, 500), 284);
        assert_eq!(pan_limit(4216, 232, 500), 4216);
    }

    #[test]
    fn native_board_locations_follow_one_note_through_navigation_removal_and_backup() -> Result<()> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!("paperroom-native-locations-{}-{nonce}", std::process::id()));
        fs::create_dir(&root)?;
        let test = || -> Result<()> {
            let path = root.join("notes.sqlite3");
            let backup = root.join("restored.sqlite3");
            let mut db = Notebook::open(&path, false)?;
            let first_parent = db.add(None, "第一個資料夾", "")?;
            let second_parent = db.add(None, "第二個資料夾", "")?;
            let note = db.add(Some(first_parent), "鏡頭", "直接來自原始分鏡的正文")?;
            let decoy = db.add(Some(second_parent), "鏡頭", "同名備用素材的正文")?;
            let collection = db.create_collection("收錄素材")?;
            let peer_collection = db.create_collection("引用素材")?;
            db.add_to_collection(collection, note, "已收錄")?;
            db.add_to_collection(peer_collection, decoy, "備用")?;
            db.add_collection_relation_between(
                peer_collection, RelationMember { note: decoy, status: "備用" }, "素材來源",
                collection, RelationMember { note, status: "已收錄" },
            )?;
            let source = root.join("備註.txt");
            fs::write(&source, "原始附件")?;
            let attachment = db.import_attachment(note, &source)?;
            fs::write(&source, "備用附件")?;
            let decoy_attachment = db.import_attachment(decoy, &source)?;
            let north = db.create_board("北板")?;
            let south = db.create_board("南板")?;
            db.add_card_on(north, note)?;
            db.add_card_on(south, decoy)?;
            db.add_card_on(south, note)?;
            db.move_card_on(north, note, 3810, 6040)?;
            db.move_card_on(south, note, 920, 395)?;
            let locations = db.native_board_locations(note)?;
            assert_eq!(locations.iter().map(|p| (p.board, p.title.as_str())).collect::<Vec<_>>(),
                [(north, "北板"), (south, "南板")]);
            assert_eq!(db.native_board_locations(decoy)?.iter().map(|p| p.board).collect::<Vec<_>>(), [south]);

            db.save(note, 1, "鏡頭已改名", "直接來自原始分鏡的正文")?;
            db.relocate(note, Some(second_parent))?;
            for location in &locations {
                let current = db.note_at_native_board_location(location)?;
                assert_eq!((current.row.id, current.row.parent, current.row.title.as_str(), current.body.as_str()),
                    (note, Some(second_parent), "鏡頭已改名", "直接來自原始分鏡的正文"));
            }
            db.backup(&backup)?;
            let backup_bytes = fs::read(&backup)?;
            let other = Notebook::open(&path, false)?;
            other.remove_card_on(north, note)?;
            assert!(db.note_at_native_board_location(&locations[0]).is_err());
            assert_eq!(db.note_at_native_board_location(&locations[1])?.row.id, note);
            assert_eq!(other.cards_on(north)?.len(), 0);
            assert_eq!(other.cards_on(south)?.len(), 2);
            other.trash(note)?;
            assert!(db.native_board_locations(note)?.is_empty());
            assert!(db.note_at_native_board_location(&locations[1]).is_err());
            drop(other);
            db.restore(note)?;
            drop(db);
            let live = Notebook::open(&path, false)?;
            let live_locations = live.native_board_locations(note)?;
            assert_eq!(live_locations.len(), 1);
            assert_eq!(live_locations[0].board, south);
            assert_eq!(live.note_at_native_board_location(&live_locations[0])?.row.id, note);

            let restored_copy = root.join("isolated.sqlite3");
            fs::copy(&backup, &restored_copy)?;
            let restored = Notebook::open(&restored_copy, false)?;
            assert_eq!(restored.native_board_locations(note)?.len(), 2);
            for (loc, expected) in locations.iter().zip([(3810, 6040), (920, 395)]) {
                assert_eq!(restored.note_at_native_board_location(loc)?.row.id, note);
                let placed = restored.cards_on(loc.board)?.into_iter().find(|card| card.id == note).unwrap();
                assert_eq!((placed.x, placed.y), expected);
            }
            for (label, db) in [("live", &live), ("restored", &restored)] {
                let origin = db.named_relation_origin(decoy)?;
                let (relations, _) = db.named_relation_page(&origin, Direction::Outgoing, 0)?;
                assert_eq!(relations.len(), 1);
                assert_eq!(db.note_from_named_relation(&origin, Direction::Outgoing, &relations[0])?.row.id, note);
                for (id, expected) in [(attachment, "原始附件"), (decoy_attachment, "備用附件")] {
                    let destination = root.join(format!("{label}-{id}.txt"));
                    db.save_attachment_copy(id, &destination)?;
                    assert_eq!(fs::read(destination)?, expected.as_bytes());
                }
            }
            drop(restored);
            assert!(fs::read(&backup)? == backup_bytes, "opening the restore copy changed its source backup");
            Ok(())
        };
        let result = test();
        fs::remove_dir_all(root)?;
        result
    }
}
