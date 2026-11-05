use rusthouse_query::{
    DerivedColumn, execute_derived_grouped_select, parse_derived_grouped_select_with_schema,
};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn filters_grouped_derived_rows_by_ratio_with_inclusive_bounds_and_nulls_first() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-derived-ratio-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let facts = root.join("facts");
    let dimensions = root.join("dimensions");
    write_wide_part(
        &facts,
        &[
            Column {
                name: "dim_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "item".into(),
                data_type: "String".into(),
            },
            Column {
                name: "period".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "before_value".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "after_value".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![
                Some("1".into()),
                Some("a".into()),
                Some("1".into()),
                Some("3".into()),
                Some("0".into()),
            ],
            vec![
                Some("1".into()),
                Some("a".into()),
                Some("2".into()),
                Some("0".into()),
                Some("2".into()),
            ],
            vec![
                Some("2".into()),
                Some("b".into()),
                Some("1".into()),
                Some("3".into()),
                Some("0".into()),
            ],
            vec![
                Some("2".into()),
                Some("b".into()),
                Some("2".into()),
                Some("0".into()),
                Some("6".into()),
            ],
            vec![
                Some("3".into()),
                Some("g".into()),
                Some("1".into()),
                Some("3".into()),
                Some("0".into()),
            ],
            vec![
                Some("3".into()),
                Some("g".into()),
                Some("2".into()),
                Some("0".into()),
                Some("4".into()),
            ],
            vec![
                Some("4".into()),
                Some("l".into()),
                Some("1".into()),
                Some("4".into()),
                Some("0".into()),
            ],
            vec![
                Some("4".into()),
                Some("l".into()),
                Some("2".into()),
                Some("0".into()),
                Some("2".into()),
            ],
            vec![
                Some("5".into()),
                Some("z".into()),
                Some("1".into()),
                Some("0".into()),
                Some("0".into()),
            ],
            vec![
                Some("5".into()),
                Some("z".into()),
                Some("2".into()),
                Some("0".into()),
                Some("5".into()),
            ],
            vec![
                Some("6".into()),
                Some("n".into()),
                Some("1".into()),
                Some("1".into()),
                Some("0".into()),
            ],
            vec![
                Some("6".into()),
                Some("n".into()),
                Some("2".into()),
                Some("0".into()),
                Some("1".into()),
            ],
            vec![
                Some("7".into()),
                Some("m".into()),
                Some("1".into()),
                Some("2".into()),
                Some("0".into()),
            ],
            vec![
                Some("7".into()),
                Some("m".into()),
                Some("2".into()),
                Some("0".into()),
                Some("3".into()),
            ],
        ],
    )
    .unwrap();
    write_wide_part(
        &dimensions,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "name".into(),
                data_type: "Nullable(String)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("Alpha".into())],
            vec![Some("2".into()), Some("Beta".into())],
            vec![Some("3".into()), Some("Gamma".into())],
            vec![Some("4".into()), Some("Low".into())],
            vec![Some("5".into()), Some("Zero".into())],
            vec![Some("6".into()), None],
            vec![Some("7".into()), Some("Max".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT * FROM (SELECT d.name, f.item, SUM(CASE WHEN f.period < 2 THEN f.before_value ELSE 0 END) AS before_total, SUM(CASE WHEN f.period >= 2 THEN f.after_value ELSE 0 END) AS after_total FROM facts f, dimensions d, dimensions d2 WHERE f.dim_id = d.id AND d.id = d2.id GROUP BY d.name, f.item) x WHERE (CASE WHEN before_total > 0 THEN (after_total * 1.000) / before_total ELSE NULL END) BETWEEN 2.000 / 3.000 AND 3.000 / 2.000 ORDER BY name NULLS FIRST, item NULLS FIRST LIMIT 100";
    let plan = parse_derived_grouped_select_with_schema(sql, |table| {
        let schema = match table {
            "facts" => vec![
                ("dim_id", "UInt64"),
                ("item", "String"),
                ("period", "UInt64"),
                ("before_value", "UInt64"),
                ("after_value", "UInt64"),
            ],
            "dimensions" => vec![("id", "UInt64"), ("name", "Nullable(String)")],
            _ => return Err(rusthouse_query::ParseError("unknown test table".into())),
        };
        Ok(schema
            .into_iter()
            .map(|(name, data_type)| DerivedColumn {
                name: name.into(),
                data_type: data_type.into(),
            })
            .collect())
    })
    .unwrap()
    .expect("bounded grouped ratio query");
    let mut output = Vec::new();
    execute_derived_grouped_select(
        &plan,
        &[facts.clone(), dimensions.clone(), dimensions.clone()],
        &mut output,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "name\titem\tbefore_total\tafter_total\n\\N\tn\t1\t1\nAlpha\ta\t3\t2\nGamma\tg\t3\t4\nMax\tm\t2\t3\n"
    );

    let sql_with_offset = sql.replace("LIMIT 100", "LIMIT 1 OFFSET 2");
    let plan_with_offset = parse_derived_grouped_select_with_schema(&sql_with_offset, |table| {
        let schema = match table {
            "facts" => vec![
                ("dim_id", "UInt64"),
                ("item", "String"),
                ("period", "UInt64"),
                ("before_value", "UInt64"),
                ("after_value", "UInt64"),
            ],
            "dimensions" => vec![("id", "UInt64"), ("name", "Nullable(String)")],
            _ => return Err(rusthouse_query::ParseError("unknown test table".into())),
        };
        Ok(schema
            .into_iter()
            .map(|(name, data_type)| DerivedColumn {
                name: name.into(),
                data_type: data_type.into(),
            })
            .collect())
    })
    .unwrap()
    .expect("grouped ratio query with offset");
    let mut output = Vec::new();
    execute_derived_grouped_select(
        &plan_with_offset,
        &[facts, dimensions.clone(), dimensions],
        &mut output,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "name\titem\tbefore_total\tafter_total\nGamma\tg\t3\t4\n"
    );
    fs::remove_dir_all(root).unwrap();
}
