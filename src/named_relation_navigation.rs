//! Browse typed relation edges from either endpoint without creating a reverse edge.
use super::{Note, Notebook, Result, read_row};
use rusqlite::{OptionalExtension, params};

pub(crate) const PAGE_SIZE: usize = 40;

#[derive(Clone, Copy)]
pub(crate) enum Direction {
    Incoming,
    Outgoing,
}

pub(crate) struct RelationOrigin {
    note: i64,
    token: String,
}

pub(crate) struct RelationHit {
    pub note: i64,
    pub title: String,
    pub location: Option<String>,
    pub name: String,
    token: String,
}

impl Notebook {
    pub(crate) fn named_relation_origin(&self, note: i64) -> Result<RelationOrigin> {
        self.connection.query_row(
            "SELECT i.token FROM notes n JOIN note_link_identities i ON i.note_id=n.id
             WHERE n.id=?1 AND n.trash_root IS NULL",
            [note], |row| Ok(RelationOrigin { note, token: row.get(0)? }),
        ).optional()?.ok_or_else(|| "找不到使用中的來源筆記。".into())
    }

    pub(crate) fn named_relation_page(
        &self, origin: &RelationOrigin, direction: Direction, page: usize,
    ) -> Result<(Vec<RelationHit>, bool)> {
        let offset = page.checked_mul(PAGE_SIZE).and_then(|offset| i64::try_from(offset).ok())
            .ok_or("關聯頁碼超過可讀取範圍。")?;
        let tx = self.connection.unchecked_transaction()?;
        if self.named_relation_origin(origin.note)?.token != origin.token {
            return Err("來源筆記已變更，請重新開啟。".into());
        }
        let (here, there) = match direction {
            Direction::Incoming => ("target_id", "source_id"),
            Direction::Outgoing => ("source_id", "target_id"),
        };
        let sql = format!(
            "SELECT n.id,n.title,CASE WHEN v.note_id IS NOT NULL THEN vr.title || ' (#' || vr.id || ') / ' || v.relative_path
                  WHEN q.note_id IS NOT NULL THEN qr.title || ' (#' || qr.id || ') / ' ||
                       case when q.archive_part='' then '' else q.archive_part || ' / ' end || q.relative_path
                  ELSE p.title END,r.name,i.token
             FROM note_relations r JOIN notes n ON n.id=r.{there}
             JOIN note_link_identities i ON i.note_id=n.id
             LEFT JOIN vault_paths v ON v.note_id=n.id
             LEFT JOIN notion_page_paths q ON q.note_id=n.id
             LEFT JOIN notes vr ON vr.id=v.import_root
             LEFT JOIN notes qr ON qr.id=q.import_root
             LEFT JOIN notes p ON p.id=n.parent
             WHERE r.{here}=?1 AND n.trash_root IS NULL
             ORDER BY r.name COLLATE BINARY,n.title COLLATE BINARY,n.id
             LIMIT ?2 OFFSET ?3"
        );
        let mut hits = {
            let mut query = tx.prepare_cached(&sql)?;
            query.query_map(params![origin.note, PAGE_SIZE as i64 + 1, offset], |row| Ok(RelationHit {
                note: row.get(0)?, title: row.get(1)?, location: row.get(2)?,
                name: row.get(3)?, token: row.get(4)?,
            }))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.commit()?;
        let more = hits.len() > PAGE_SIZE;
        hits.truncate(PAGE_SIZE);
        Ok((hits, more))
    }

    /// Resolve the exact directed edge and both active identities in one statement.
    pub(crate) fn note_from_named_relation(
        &self, origin: &RelationOrigin, direction: Direction, hit: &RelationHit,
    ) -> Result<Note> {
        let (from, to) = match direction {
            Direction::Incoming => (hit.note, origin.note),
            Direction::Outgoing => (origin.note, hit.note),
        };
        self.connection.query_row(
            "SELECT n.id,n.parent,n.title,n.pinned,n.trash_root,n.body,n.version
             FROM note_relations r
             JOIN notes origin ON origin.id=?1
             JOIN note_link_identities oi ON oi.note_id=origin.id
             JOIN notes n ON n.id=?2
             JOIN note_link_identities i ON i.note_id=n.id
             WHERE r.source_id=?3 AND r.target_id=?4 AND r.name=?5
               AND oi.token=?6 AND i.token=?7
               AND origin.trash_root IS NULL AND n.trash_root IS NULL",
            params![origin.note, hit.note, from, to, hit.name, origin.token, hit.token],
            |row| Ok(Note { row: read_row(row)?, body: row.get(5)?, version: row.get(6)? }),
        ).optional()?.ok_or_else(|| "關聯或來源、目標筆記已變更，請重新開啟。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use crate::store::collections::RelationMember;
    use std::{fs, path::Path, time::{SystemTime, UNIX_EPOCH}};

    #[test]
    fn shared_edge_matches_the_link_canvas_and_backup_after_rename_and_trash() -> Result<()> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir = std::env::temp_dir().join(format!("paperroom-unified-{}-{nonce}", std::process::id()));
        fs::create_dir(&dir)?;
        let work = || -> Result<()> {
            let original = dir.join("notes.sqlite3");
            let backup = dir.join("restored.sqlite3");
            let mut db = Notebook::open(&original, false)?;
            let source = db.add(None, "分鏡計畫", "計畫唯一正文")?;
            let vault = db.import_vault("研究用 Vault", vec![
                ImportedPage { parts: vec!["片場".into(), "鏡頭.md".into()],
                    title: "鏡頭".into(), body: "片場唯一正文".into() },
                ImportedPage { parts: vec!["備用".into(), "鏡頭.md".into()],
                    title: "鏡頭".into(), body: "備用唯一正文".into() },
            ])?;
            let target_hits = db.quick_notes("片場/鏡頭.md")?;
            let decoy_hits = db.quick_notes("備用/鏡頭.md")?;
            assert_eq!((target_hits.len(), decoy_hits.len()), (1, 1));
            let (target, decoy) = (target_hits[0].id, decoy_hits[0].id);
            assert_ne!(target, decoy);
            db.save(source, 1, "分鏡計畫", &db.markup_for_picked_note(&target_hits[0])?)?;

            let group_a = db.create_collection("計畫")?;
            let group_b = db.create_collection("素材")?;
            db.add_to_collection(group_a, source, "待拍")?;
            for note in [target, decoy] { db.add_to_collection(group_b, note, "已收錄")?; }
            db.add_collection_relation_between(
                group_a, RelationMember { note: source, status: "待拍" }, "使用",
                group_b, RelationMember { note: target, status: "已收錄" },
            )?;
            db.compare_set_property(target, "秒數", None, Some("12"))?;
            db.compare_set_property(decoy, "秒數", None, Some("8"))?;
            assert_eq!(db.rollup_sum(source, "使用", "秒數")?, 12.0);

            let shared = dir.join("鏡頭.txt");
            fs::write(&shared, "片場附件")?;
            let target_attachment = db.import_attachment(target, &shared)?;
            fs::write(&shared, "備用附件")?;
            let decoy_attachment = db.import_attachment(decoy, &shared)?;
            let node = |id| serde_json::json!({"type":"file","id":id,"file":"片場/鏡頭.md",
                "x":0,"y":0,"width":120,"height":80});
            let north = db.save_canvas("北", &crate::canvas::parse(
                serde_json::json!({"nodes":[node("first"),node("second")]}).to_string(),
            )?)?;
            let south = db.save_canvas("南", &crate::canvas::parse(
                serde_json::json!({"nodes":[node("only")]}).to_string(),
            )?)?;
            for board in [north, south] { db.set_canvas_vault(board, Some(vault))?; }

            let source_origin = db.named_relation_origin(source)?;
            let target_origin = db.named_relation_origin(target)?;
            let (outgoing, false) = db.named_relation_page(&source_origin, Direction::Outgoing, 0)?
                else { panic!("one outgoing page expected"); };
            assert_eq!(outgoing.len(), 1);
            assert_eq!((outgoing[0].title.as_str(), outgoing[0].name.as_str()), ("鏡頭", "使用"));
            assert!(outgoing[0].location.as_deref().unwrap().ends_with("片場/鏡頭.md"));
            let (backlinks, false) = db.named_relation_page(&target_origin, Direction::Incoming, 0)?
                else { panic!("one incoming page expected"); };
            assert_eq!(backlinks.len(), 1);
            assert!(db.named_relation_page(&target_origin, Direction::Outgoing, 0)?.0.is_empty());
            assert!(db.named_relation_page(&db.named_relation_origin(decoy)?, Direction::Incoming, 0)?.0.is_empty());

            db.save(target, 1, "鏡頭已改名", "片場唯一正文")?;
            let moved = db.add(Some(vault), "移動後", "")?;
            db.relocate(target, Some(moved))?;
            let next = db.note_from_named_relation(&source_origin, Direction::Outgoing, &outgoing[0])?;
            assert_eq!((next.row.id, next.row.parent, next.row.title.as_str(), next.body.as_str()),
                (target, Some(moved), "鏡頭已改名", "片場唯一正文"));
            db.compare_set_property(target, "秒數", Some("12"), Some("6"))?;
            assert_eq!(db.rollup_sum(source, "使用", "秒數")?, 6.0);

            db.trash(target)?;
            assert!(db.note_from_named_relation(&source_origin, Direction::Outgoing, &outgoing[0]).is_err());
            assert!(db.named_relation_page(&target_origin, Direction::Incoming, 0).is_err());
            assert!(db.canvas_note_placements(target)?.is_empty());
            assert_eq!(db.rollup_count(source, "使用")?, 0);
            db.restore(target)?;
            assert_eq!(db.note_from_named_relation(&target_origin, Direction::Incoming, &backlinks[0])?.row.id, source);
            assert!(db.note_from_named_relation(&target_origin, Direction::Outgoing, &backlinks[0]).is_err());
            assert_eq!(db.rollup_sum(source, "使用", "秒數")?, 6.0);
            db.set_relation(source, "使用", target, false)?;
            assert!(db.note_from_named_relation(&source_origin, Direction::Outgoing, &outgoing[0]).is_err());
            db.set_relation(source, "使用", target, true)?;
            drop(db);

            let reopened = Notebook::open(&original, false)?;
            reopened.backup(&backup)?;
            drop(reopened);
            let review = Notebook::open(&backup, false)?;
            let recovered = review.note_from_named_relation(&source_origin, Direction::Outgoing, &outgoing[0])?;
            assert_eq!(recovered.row.id, target);
            assert_eq!(review.note_from_named_relation(&target_origin, Direction::Incoming, &backlinks[0])?.row.id, source);
            let document = review.note(source)?.unwrap();
            assert_eq!(review.vault_note_links(&document)?[0].note_id, target);
            let (linked, backlinks) = review.graph_neighbors(&recovered)?;
            assert!(linked.is_empty());
            assert_eq!(backlinks.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(), [source]);
            let placements = review.canvas_note_placements(target)?;
            assert_eq!(placements.len(), 2);
            for placement in placements {
                let expected: &[&str] = if placement.canvas_id == north { &["first", "second"] } else {
                    assert_eq!(placement.canvas_id, south);
                    &["only"]
                };
                let occurrences = review.canvas_note_occurrences(target, &placement)?;
                assert_eq!(occurrences.iter().map(|hit| hit.link.node_id.as_str()).collect::<Vec<_>>(), expected);
                for occurrence in occurrences {
                    review.locate_canvas_note(target, &placement, &occurrence)?;
                    assert_eq!(review.resolve_canvas_file_link(placement.canvas_id, Some(vault),
                        &occurrence.link.node_id)?.note_id, Some(target));
                }
            }
            for (id, name, expected) in [(target_attachment, "target.txt", "片場附件"),
                (decoy_attachment, "decoy.txt", "備用附件")] {
                let output = dir.join(name);
                review.save_attachment_copy(id, &output)?;
                assert_eq!(fs::read(output)?, expected.as_bytes());
            }
            assert_eq!(review.rollup_sum(source, "使用", "秒數")?, 6.0);
            Ok(())
        };
        let result = work();
        fs::remove_dir_all(dir)?;
        result
    }

    #[test]
    fn incoming_relations_page_across_many_sources() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let target = db.add(None, "目標", "")?;
        let origin = db.named_relation_origin(target)?;
        let limit = PAGE_SIZE + 2;
        let mut sources = Vec::new();
        for i in 0..limit {
            let source = db.add(None, &format!("來源 {i:03}"), "")?;
            db.set_relation(source, "來源關聯", target, true)?;
            sources.push(source);
        }
        let (first, more) = db.named_relation_page(&origin, Direction::Incoming, 0)?;
        assert!(more);
        assert_eq!(first.iter().map(|hit| hit.note).collect::<Vec<_>>(), sources[..PAGE_SIZE]);
        let (last, more) = db.named_relation_page(&origin, Direction::Incoming, 1)?;
        assert!(!more);
        assert_eq!(last.iter().map(|hit| hit.note).collect::<Vec<_>>(), sources[PAGE_SIZE..]);
        assert!(db.named_relation_page(&origin, Direction::Incoming, 2)?.0.is_empty());
        Ok(())
    }
}
