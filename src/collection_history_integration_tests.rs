use super::*;
use crate::{archive::notion_archive_with_media, store::collection_group_folds::GroupValue};
use std::{fs, io::Write};
use zip::{ZipWriter, write::SimpleFileOptions};

#[test]
fn shared_import_live_views_and_attachment_history_survive_isolated_restore() -> Result<()> {
    let dir = crate::store::tests::data_dir();
    let result = (|| -> Result<()> {
        let project = "Project 11111111111111111111111111111111";
        let child = "Child 22222222222222222222222222222222";
        let catalog = "Dates 33333333333333333333333333333333_all.csv";
        let asset = b"Original Notion attachment.\n";
        let today = CalendarDay::local_today()?;
        let due = today.shifted(1).unwrap().iso();
        let second_day = today.shifted(2).unwrap().iso();
        let later = today.shifted(8).unwrap().iso();
        let csv = format!("Name,Bucket,Due,Memo\nShared,Hidden,{due},Old text\nCover,Open,{due},Control A\n\
                           Side,Side,{due},Control B\nWitness,Open,{second_day},Control C\n");
        let archive = dir.join("source.zip");
        let source_link = format!("# Project\n\n[Open](Project%2011111111111111111111111111111111/Child%2022222222222222222222222222222222.md)\n");
        {
            let mut zip = ZipWriter::new(fs::File::create_new(&archive)?);
            for (path, bytes) in [
                (format!("{project}.md"), source_link.as_bytes()),
                (format!("{project}/{child}.md"), b"# Native asset page\n\n[Evidence](source.txt)\n"),
                (format!("{project}/source.txt"), asset),
                (format!("{project}/{catalog}"), csv.as_bytes()),
            ] {
                zip.start_file(path, SimpleFileOptions::default())?;
                zip.write_all(bytes)?;
            }
            zip.finish()?;
        }
        let archive_before = fs::read(&archive)?;
        let path = dir.join("main.db");
        let mut db = Notebook::open(&path, false)?;
        let (imported, media, notion_csv) = notion_archive_with_media(&archive)?;
        db.import_notion_with_media("Workspace import", imported.pages, media, notion_csv)?;
        let (source, original_csv): (i64,i64) = db.connection.query_row(
            "SELECT note_id,id FROM attachments WHERE name=?1", [catalog], |r| Ok((r.get(0)?,r.get(1)?)),
        )?;
        let original_asset: i64 = db.connection.query_row(
            "SELECT id FROM attachments WHERE name='source.txt'", [], |r| r.get(0),
        )?;
        let (a, count) = db.create_collection_from_notion_csv(source, original_csv, "A-Review")?;
        assert_eq!(count, 4);
        let b = db.create_collection("B-Review")?;
        let rows: HashMap<_,_> = db.collection_rows(a)?.into_iter().map(|(row, _)| (row.title, row.id)).collect();
        for note in rows.values() { db.add_to_collection(b, *note, "待處理")?; }
        db.compare_set_property_type("Due", None, Some(PropertyType::Date))?;
        let note = rows["Shared"];
        let original = dir.join("Clip.txt");
        let replacement = dir.join("swap");
        fs::create_dir(&replacement)?;
        let replacement = replacement.join("Clip.txt");
        fs::write(&original, b"ORIG")?;
        fs::write(&replacement, b"SWAP")?;
        let original_id = db.import_attachment(note, &original)?;

        let rules_for = |operator| ViewRules {
            filters: vec![FilterRule { key:"Due".into(), kind:FilterKind::Typed(PropertyType::Date),
                operator, value:String::new(), argument:None }],
            ..Default::default()
        };
        let configure = |collection, name, operator, fields: Vec<String>, group: &str| -> Result<CollectionView> {
            let view = db.save_presented_collection_view(collection, name, CollectionLayout::Table, "", "", None,
                Presentation { group_key:Some("Bucket".into()), ..Default::default() })?;
            db.set_collection_display_columns(collection,None,Some(&fields))?;
            db.set_collection_view_rules(collection,&view,rules_for(operator))?;
            let fresh = db.collection_views(collection)?.remove(0);
            let folds = db.collection_group_folds(&fresh)?.unwrap();
            db.set_collection_group_fold(collection,&fresh,&folds,Some((&GroupValue(Some(group.into())),true)))?;
            Ok(fresh)
        };
        let a_fields = vec!["Due".into(), "Memo".into()];
        let b_fields = vec!["Memo".into(), "Due".into()];
        let first = configure(a,"A-Timing",FilterOperator::NextSevenDays,a_fields.clone(),"Hidden")?;
        let second = configure(b,"B-Timing",FilterOperator::Tomorrow,b_fields.clone(),"Side")?;
        let hidden = GroupValue(Some("Hidden".into()));
        let side = GroupValue(Some("Side".into()));
        let check_views = |db: &Notebook| -> Result<()> {
            assert_eq!(db.collection_views(a)?, [first.clone()]);
            assert_eq!(db.collection_views(b)?, [second.clone()]);
            assert_eq!(db.collection_display_columns(a)?, Some(a_fields.clone()));
            assert_eq!(db.collection_display_columns(b)?, Some(b_fields.clone()));
            let fa = db.collection_group_folds(&first)?.unwrap();
            let fb = db.collection_group_folds(&second)?.unwrap();
            assert!(fa.contains(&hidden) && !fa.contains(&side));
            assert!(fb.contains(&side) && !fb.contains(&hidden));
            assert_eq!(db.collection_rows(a)?.len(), 4);
            assert_eq!(db.collection_rows(b)?.len(), 4);
            Ok(())
        };
        let shown = |db: &Notebook, col, view: &CollectionView| -> Result<Vec<String>> {
            let folds = db.collection_group_folds(view)?.unwrap();
            Ok(db.presented_collection_lanes(col,Some(view))?.into_iter()
                .filter(|(group,_)| !folds.contains(group)).flat_map(|(_,rows)|rows.into_iter().map(|row|row.title))
                .collect())
        };
        let verify_bytes = |db: &Notebook, id, name, expected: &[u8]| -> Result<()> {
            let exported = dir.join(name);
            db.save_attachment_copy(id, &exported)?;
            assert_eq!(fs::read(exported)?, expected);
            Ok(())
        };
        check_views(&db)?;
        assert_eq!(shown(&db,a,&first)?, ["Cover","Witness","Side"]);
        assert_eq!(shown(&db,b,&second)?, ["Shared","Cover"]);
        db.compare_set_property(note,"Due",Some(&due),Some(&later))?;
        let target = db.note_revisions(note)?[0].id;
        db.remove_attachment(original_id)?;
        let swap_id = db.import_attachment(note,&replacement)?;
        check_views(&db)?;
        assert_eq!(shown(&db,b,&second)?, ["Cover"]);
        verify_bytes(&db,swap_id,"during.txt",b"SWAP")?;
        let review = db.preview_note_revision(note,target)?;
        assert!(review.metadata_changed && review.metadata.contains(&format!("Due = {due}")));
        assert!(review.metadata.contains(&format!("Clip.txt　4 位元組　#{original_id}")));
        assert!(db.restore_note_revision(note,db.note(note)?.unwrap().version,target,Some(review.data_version))?);
        check_views(&db)?;
        assert_eq!(shown(&db,a,&first)?, ["Cover","Witness","Side"]);
        assert_eq!(shown(&db,b,&second)?, ["Shared","Cover"]);
        verify_bytes(&db,original_id,"after.txt",b"ORIG")?;
        drop(db);

        let db = Notebook::open(&path,false)?;
        check_views(&db)?;
        verify_bytes(&db,original_id,"reopened.txt",b"ORIG")?;
        let backup = dir.join("full-backup.db");
        db.backup(&backup)?;
        drop(db);
        let source_before = fs::read(&path)?;
        let backup_before = fs::read(&backup)?;
        let isolated = dir.join("isolated.db");
        fs::copy(&backup,&isolated)?;
        let restored = Notebook::open(&isolated,false)?;
        check_views(&restored)?;
        verify_bytes(&restored,original_id,"restored.txt",b"ORIG")?;
        verify_bytes(&restored,original_csv,"csv-copy.csv",csv.as_bytes())?;
        verify_bytes(&restored,original_asset,"archive-asset.txt",asset)?;
        let current = restored.collection_group_folds(&first)?.unwrap();
        restored.set_collection_group_fold(a,&first,&current,Some((&hidden,false)))?;
        assert_eq!(shown(&restored,a,&first)?, ["Shared","Cover","Witness","Side"]);
        let current = restored.collection_group_folds(&first)?.unwrap();
        restored.set_collection_group_fold(a,&first,&current,Some((&hidden,true)))?;
        check_views(&restored)?;

        // Real metadata saves rotate the 30-revision budget on the restored copy.
        let displaced = restored.note_revisions(note)?[0].id;
        assert!(restored.preview_note_revision(note,displaced)?.metadata.contains(&format!("#{swap_id}")));
        let mut old = "Old text".to_string();
        for tick in 1..=30 {
            let next = format!("Saved edit {tick}");
            restored.compare_set_property(note,"Memo",Some(&old),Some(&next))?;
            old = next;
        }
        assert_eq!(restored.note_revisions(note)?.len(),30);
        assert!(restored.preview_note_revision(note,displaced).is_err());
        assert!(restored.restore_note_revision(note,restored.note(note)?.unwrap().version,displaced,None).is_err());
        let history_files: i64 = restored.connection.query_row(
            "SELECT count(*) FROM note_revision_attachments WHERE original_id=?1", [swap_id], |r| r.get(0),
        )?;
        assert_eq!(history_files,0);
        verify_bytes(&restored,original_id,"after-retention.txt",b"ORIG")?;
        check_views(&restored)?;
        assert_eq!(shown(&restored,b,&second)?, ["Shared","Cover"]);
        assert_eq!(fs::read(path)?,source_before);
        assert_eq!(fs::read(backup)?,backup_before);
        assert_eq!(fs::read(archive)?,archive_before);
        Ok(())
    })();
    let _ = fs::remove_dir_all(dir);
    result
}
