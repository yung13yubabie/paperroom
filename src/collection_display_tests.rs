use super::*;
use crate::store::{View, collection_views::{CollectionLayout, PropertyFilter}, typed_properties::PropertyType};
use std::fs;

#[test]
fn display_order_is_independent_of_shared_values_and_survives_backup() -> Result<()> {
    let dir = crate::store::tests::data_dir();
    let result = (|| -> Result<()> {
        let source = dir.join("source.db");
        let mut db = Notebook::open(&source, false)?;
        let left = db.create_collection("甲集合")?;
        let right = db.create_collection("乙集合")?;
        let csv = "標題,項目,工時,備註\n共享,搶救,003.50,保留原稿\n".as_bytes();
        let (_, count) = db.import_csv("匯入欄位", csv)?;
        assert_eq!(count, 1);
        let row = db.rows(View::All, "共享")?.remove(0).id;
        for collection in [left, right] { db.add_to_collection(collection, row, "待處理")?; }
        db.compare_set_property_type("工時", None, Some(PropertyType::Number))?;
        db.compare_set_property_type("尚未給值", None, Some(PropertyType::Date))?;
        let filter = PropertyFilter { key:"工時".into(), kind:PropertyType::Number, value:"3.5".into() };
        let view = db.save_collection_view(left, "工時表格", CollectionLayout::Table, "", "", Some(filter))?;
        db.connection.execute("DROP TABLE collection_display_columns", [])?;
        db.connection.pragma_update(None, "user_version", 33)?;
        drop(db);

        let db = Notebook::open(&source, false)?;
        assert_eq!(db.collection_display_columns(left)?, None);
        assert_eq!(db.collection_display_columns(right)?, None);
        assert_eq!(db.collection_display_keys(left, "目")?, (vec!["項目".into()], false));
        assert!(db.collection_display_keys(right, "")?.0.contains(&"尚未給值".into()));

        let columns = |keys: &[&str]| keys.iter().map(|key| key.to_string()).collect::<Vec<_>>();
        let ordered = columns(&["項目", "工時"]);
        let reordered = columns(&["工時", "項目"]);
        db.set_collection_display_columns(left, None, Some(&ordered))?;
        db.set_collection_display_columns(right, None, Some(&[]))?;
        assert_eq!(db.collection_display_columns(left)?, Some(ordered.clone()));
        assert_eq!(db.collection_display_columns(right)?, Some(vec![]));
        assert_eq!(db.collection_property_values(left, "工時")?[&row], "003.50");
        assert_eq!(db.collection_property_values(right, "工時")?[&row], "003.50");
        let previous_properties = db.properties(row)?;
        let previous_history = db.note_revisions(row)?.len();

        let peer = Notebook::open(&source, false)?;
        peer.set_collection_display_columns(left, Some(&ordered), Some(&reordered))?;
        assert!(db.set_collection_display_columns(left, Some(&ordered), None).is_err());
        for invalid in [
            columns(&["不存在"]), columns(&["項目","項目"]), columns(&["工時","備註","項目","標題","尚未給值"]),
        ] {
            assert!(db.set_collection_display_columns(left, Some(&reordered), Some(&invalid)).is_err());
            assert_eq!(db.collection_display_columns(left)?, Some(reordered.clone()));
        }
        drop(peer);
        db.set_collection_display_columns(right, Some(&[]), None)?;
        db.set_collection_display_columns(right, None, Some(&columns(&["備註"])))?;
        assert_eq!(db.collection_views(left)?, [view.clone()]);
        assert_eq!(db.filtered_collection_rows(left, Some(&view))?[0].0.id, row);
        assert_eq!(db.properties(row)?, previous_properties);
        assert_eq!(db.note_revisions(row)?.len(), previous_history);
        drop(db);

        let db = Notebook::open(&source, false)?;
        let archive = dir.join("archive.db");
        db.backup(&archive)?;
        drop(db);
        let original = fs::read(&source)?;
        let isolated = dir.join("isolated.db");
        fs::copy(&archive, &isolated)?;
        let copy = Notebook::open(&isolated, false)?;
        assert_eq!(copy.collection_display_columns(left)?, Some(reordered.clone()));
        assert_eq!(copy.collection_display_columns(right)?, Some(columns(&["備註"])));
        assert_eq!(copy.collection_views(left)?, [view]);
        assert_eq!(copy.properties(row)?, previous_properties);
        copy.set_collection_display_columns(left, Some(&reordered), None)?;
        assert_eq!(copy.collection_display_columns(left)?, None);
        assert_eq!(fs::read(source)?, original);
        Ok(())
    })();
    let _ = fs::remove_dir_all(dir);
    result
}
