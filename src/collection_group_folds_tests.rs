use super::*;
use crate::store::{
    collection_views::{FilterKind, FilterOperator, FilterRule, Presentation, SortRule, SortSource, ViewRules},
    typed_properties::PropertyType,
};
use std::fs;

#[test]
fn isolated_group_values_remain_folded_through_rule_changes_reopen_and_backup() -> Result<()> {
    let dir = crate::store::tests::data_dir();
    let result = (|| -> Result<()> {
        let source = dir.join("source.db");
        let mut db = Notebook::open(&source, false)?;
        let first = db.create_collection("甲")?;
        let second = db.create_collection("乙")?;
        let group_key = "分類";
        let mut ids = Vec::new();
        for (title, group, rank) in [
            ("Empty", None, "3"), ("Literal", Some("未設定"), "2"),
            ("Early", Some("路線"), "1"), ("Late", Some("路線"), "9"),
            ("Tabs", Some("A\tB"), "4"), ("Spaces", Some("A B"), "5"),
        ] {
            let id = db.add(None, title, "")?;
            for col in [first, second] { db.add_to_collection(col, id, "待處理")?; }
            if let Some(group) = group { db.compare_set_property(id, group_key, None, Some(group))?; }
            db.compare_set_property(id, "另一欄", None, Some("未設定"))?;
            db.compare_set_property(id, "順序", None, Some(rank))?;
            ids.push(id);
        }
        db.compare_set_property_type("順序", None, Some(PropertyType::Number))?;
        let create = |db: &Notebook, col, name, layout, key: Option<&str>| db.save_presented_collection_view(
            col, name, layout, "", "", None,
            Presentation { group_key:key.map(str::to_owned), ..Default::default() },
        );
        let table = create(&db, first, "表格", CollectionLayout::Table, Some(group_key))?;
        let other_view = create(&db, first, "另一檢視", CollectionLayout::Board, Some(group_key))?;
        let other_collection = create(&db, second, "表格", CollectionLayout::Table, Some(group_key))?;
        let status = create(&db, first, "狀態看板", CollectionLayout::Board, None)?;
        let flat = create(&db, first, "平面表格", CollectionLayout::Table, None)?;
        assert!(db.collection_group_folds(&flat)?.is_none());
        db.set_collection_display_columns(first, None, Some(&["順序".into()]))?;
        db.connection.execute_batch(
            "DROP TRIGGER reset_collection_group_folds; DROP TABLE collection_group_folds; PRAGMA user_version=34;",
        )?;
        drop(db);

        let db = Notebook::open(&source, false)?;
        let state = |db: &Notebook, view: &CollectionView| db.collection_group_folds(view).map(|folds| folds.unwrap());
        let missing = GroupValue(None);
        let literal = GroupValue(Some("未設定".into()));
        let route = GroupValue(Some("路線".into()));
        let tabs = GroupValue(Some("A\tB".into()));
        let spaces = GroupValue(Some("A B".into()));
        assert_eq!(tabs.label(), spaces.label());
        assert_ne!(tabs.fold_label(), spaces.fold_label());
        for group in [&missing, &route, &tabs] {
            db.set_collection_group_fold(first, &table, &state(&db, &table)?, Some((group,true)))?;
        }
        assert!(!state(&db, &table)?.contains(&literal));
        assert!(!state(&db, &table)?.contains(&spaces));
        assert!(!state(&db, &other_view)?.contains(&route));
        assert!(!state(&db, &other_collection)?.contains(&route));
        let status_value = GroupValue(Some("待處理".into()));
        db.set_collection_group_fold(first, &status, &state(&db, &status)?, Some((&status_value,true)))?;
        assert!(state(&db, &status)?.contains(&status_value));
        let rules = ViewRules {
            filters: vec![FilterRule {
                key:"".into(), kind:FilterKind::Title, operator:FilterOperator::NotEqual, value:"Empty".into(), argument:None,
            }],
            sorts: vec![
                SortRule { key:"順序".into(), source:SortSource::Property(Some(PropertyType::Number)), descending:true },
                SortRule { key:"另一欄".into(), source:SortSource::Property(None), descending:false },
            ],
            ..Default::default()
        };
        let old = state(&db, &table)?;
        db.set_collection_view_rules(first, &table, rules.clone())?;
        assert!(db.set_collection_group_fold(first, &table, &old, Some((&literal,true))).is_err());
        let updated = |db: &Notebook| db.collection_views(first).unwrap().into_iter().find(|v| v.id == table.id).unwrap();
        let view = updated(&db);
        let lanes = db.presented_collection_lanes(first, Some(&view))?;
        assert!(!lanes.iter().any(|(group, _)| *group == missing));
        assert_eq!(lanes.iter().find(|(group, _)| *group == route).unwrap().1.iter().map(|row| row.id)
            .collect::<Vec<_>>(), [ids[3], ids[2]]);
        assert!(state(&db, &view)?.contains(&missing));
        let backup = dir.join("backup.db");
        db.backup(&backup)?;
        drop(db);
        let db = Notebook::open(&source, false)?;
        assert!(state(&db, &updated(&db))?.contains(&route));
        drop(db);
        let original = fs::read(&source)?;
        let copy = dir.join("copy.db");
        fs::copy(&backup, &copy)?;
        let db = Notebook::open(&copy, false)?;
        let view = updated(&db);
        assert_eq!(view.rules(), rules);
        assert_eq!(db.collection_display_columns(first)?, Some(vec!["順序".into()]));
        assert!(state(&db, &view)?.contains(&missing));
        assert!(state(&db, &view)?.contains(&route));
        let before_type = state(&db, &view)?;
        db.compare_set_property_type(group_key, None, Some(PropertyType::Number))?;
        assert!(db.set_collection_group_fold(first, &view, &before_type, Some((&route,true))).is_err());
        assert!(!state(&db, &view)?.contains(&route));
        assert!(state(&db, &status)?.contains(&status_value));
        db.compare_set_property_type(group_key, Some(PropertyType::Number), None)?;
        assert!(!state(&db, &view)?.contains(&route));
        db.set_collection_group_fold(first, &view, &state(&db, &view)?, Some((&literal,true)))?;
        db.connection.execute("UPDATE collection_views SET group_key='另一欄' WHERE id=?1", [view.id])?;
        let changed = updated(&db);
        assert!(!state(&db, &changed)?.contains(&literal));
        db.set_collection_group_fold(first, &changed, &state(&db, &changed)?, Some((&literal,true)))?;
        db.set_collection_group_fold(first, &changed, &state(&db, &changed)?, None)?;
        assert!(!state(&db, &changed)?.contains(&literal));
        assert_eq!(fs::read(source)?, original);
        Ok(())
    })();
    let _ = fs::remove_dir_all(dir);
    result
}
