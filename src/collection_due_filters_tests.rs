use super::*;
use std::fs;

#[test]
fn saved_due_filters_upgrade_recompute_and_survive_full_backup() -> Result<()> {
    let dir = crate::store::tests::data_dir();
    let result = (|| -> Result<()> {
        let source = dir.join("source.db");
        let mut db = Notebook::open(&source, false)?;
        let collection = db.create_collection("時程")?;
        let field = "截止日";
        let formula = "公式截止";
        let mut ids = Vec::new();
        for (title, date) in [
            ("前一週", Some("2028-12-24")),
            ("週一", Some("2028-12-25")),
            ("今天", Some("2028-12-31")),
            ("明天", Some("2029-01-01")),
            ("第七天", Some("2029-01-07")),
            ("第八天", Some("2029-01-08")),
            ("沒有日期", None),
            ("無效舊值", Some("2028-12-32")),
        ] {
            let id = db.add(None, title, "")?;
            db.add_to_collection(collection, id, "待處理")?;
            if let Some(date) = date { db.compare_set_property(id, field, None, Some(date))?; }
            ids.push(id);
        }
        db.compare_set_property_type(field, None, Some(PropertyType::Date))?;
        db.add_collection_formula(collection, formula, r#"typed("截止日")"#)?;

        let rules = |key: &str, kind, operator| ViewRules {
            filters: vec![FilterRule { key:key.into(), kind, operator, value:String::new(), argument:None }],
            ..Default::default()
        };
        let typed = FilterKind::Typed(PropertyType::Date);
        let date_formula = FilterKind::Formula(Some(PropertyType::Date));
        let week = db.create_collection_view(collection, "本週", CollectionLayout::Table, "", "")?;
        let week_rules = rules(field, typed, FilterOperator::ThisWeek);
        db.set_collection_view_rules(collection, &week, week_rules.clone())?;
        assert_eq!(week_rules.encode(), r#"[false,[["截止日","date",17,""]],[]]"#);
        db.connection.execute("DROP TABLE collection_display_columns", [])?;
        db.connection.pragma_update(None, "user_version", 32)?;
        drop(db);

        let db = Notebook::open(&source, false)?;
        let saved = |db: &Notebook, name: &str| db.collection_views(collection).unwrap().into_iter()
            .find(|view| view.name == name).unwrap();
        let results = |db: &Notebook, view: &CollectionView, date: &str| -> Result<Vec<i64>> {
            let mut result = db.filter_collection_rows(collection, Some(view), None, CalendarDay::parse(date))?
                .into_iter().map(|(row, _)| row.id).collect::<Vec<_>>();
            result.sort_unstable();
            Ok(result)
        };
        assert_eq!(db.connection.pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))?, crate::store::SCHEMA_VERSION);
        let legacy = saved(&db, "本週");
        assert_eq!(legacy.rules(), week_rules);
        assert_eq!(results(&db, &legacy, "2028-12-31")?, [ids[1], ids[2]]);

        let next = db.create_collection_view(collection, "未來七天", CollectionLayout::Table, "", "")?;
        let overdue = db.create_collection_view(collection, "逾期", CollectionLayout::Table, "", "")?;
        let next_rules = rules(field, typed, FilterOperator::NextSevenDays);
        let overdue_rules = rules(formula, date_formula, FilterOperator::Overdue);
        assert!(db.set_collection_view_rules(collection, &next, rules("", FilterKind::Title, FilterOperator::NextSevenDays)).is_err());
        assert_eq!(saved(&db, "未來七天").rules(), ViewRules::default());
        db.set_collection_view_rules(collection, &next, next_rules.clone())?;
        db.set_collection_view_rules(collection, &overdue, overdue_rules.clone())?;
        drop(db);

        let db = Notebook::open(&source, false)?;
        let check = |db: &Notebook| -> Result<()> {
            let next = saved(db, "未來七天");
            let overdue = saved(db, "逾期");
            assert_eq!(next.rules(), next_rules);
            assert_eq!(overdue.rules(), overdue_rules);
            assert_eq!(results(db, &next, "2028-12-31")?, [ids[3], ids[4]]);
            assert_eq!(results(db, &next, "2029-01-01")?, [ids[4], ids[5]]);
            assert_eq!(results(db, &overdue, "2028-12-31")?, [ids[0], ids[1]]);
            assert_eq!(results(db, &overdue, "2029-01-01")?, [ids[0], ids[1], ids[2]]);
            assert!(results(db, &overdue, "0001-01-01")?.is_empty());
            assert!(results(db, &next, "9999-12-31")?.is_empty());
            Ok(())
        };
        check(&db)?;
        let archive = dir.join("archive.db");
        db.backup(&archive)?;
        drop(db);
        let original = fs::read(&source)?;
        let isolated = dir.join("isolated.db");
        fs::copy(&archive, &isolated)?;
        let restored = Notebook::open(&isolated, false)?;
        check(&restored)?;
        restored.compare_set_property(ids[5], field, Some("2029-01-08"), Some("2028-12-30"))?;
        assert_eq!(results(&restored, &saved(&restored, "逾期"), "2028-12-31")?, [ids[0], ids[1], ids[5]]);
        assert_eq!(results(&restored, &saved(&restored, "本週"), "2028-12-31")?, [ids[1], ids[2], ids[5]]);
        assert_eq!(fs::read(source)?, original);
        Ok(())
    })();
    let _ = fs::remove_dir_all(dir);
    result
}
