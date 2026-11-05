use rusthouse_query::{
    execute_derived_grouped_select, execute_multi_join, parse_derived_grouped_select_with_schema,
    parse_join,
};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_join_tracks_multiple_nullable_avg_denominators() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-multi-avg-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let dimension = root.join("dimension");
    let dates = root.join("dates");
    let key = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    write_wide_part(
        &fact,
        &key,
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &key,
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();
    write_wide_part(
        &dimension,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "quantity".into(),
                data_type: "Nullable(UInt64)".into(),
            },
            Column {
                name: "price".into(),
                data_type: "Nullable(UInt64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into()), Some("10".into())],
            vec![Some("1".into()), Some("3".into()), None],
            vec![Some("2".into()), None, Some("6".into())],
        ],
    )
    .unwrap();
    let paths = [fact, dimension, dates];
    let sql = "SELECT f.id, AVG(d.quantity) AS avg_quantity, AVG(d.price) AS avg_price FROM fact f JOIN dimension d ON f.id=d.id JOIN dates t ON d.id=t.id GROUP BY f.id ORDER BY f.id";
    let query = parse_join(sql).unwrap().unwrap();
    assert_eq!(query.grouped_avg_metrics.len(), 2);
    let mut out = Vec::new();
    execute_multi_join(&query, &paths, &mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "id\tavg_quantity\tavg_price\n1\t2\t10\n2\t\\N\t6\n"
    );

    let query = parse_join(&format!("{sql} LIMIT 1 OFFSET 1"))
        .unwrap()
        .unwrap();
    let mut out = Vec::new();
    execute_multi_join(&query, &paths, &mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "id\tavg_quantity\tavg_price\n2\t\\N\t6\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn mixed_grouped_metrics_keep_values_and_ignore_null_avg_inputs() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-mixed-avg-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let dimension = root.join("dimension");
    let dates = root.join("dates");
    write_wide_part(
        &fact,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "group_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "min_value".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "max_value".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "avg_value".into(),
                data_type: "Nullable(UInt64)".into(),
            },
        ],
        &[
            vec![
                Some("1".into()),
                Some("1".into()),
                Some("9".into()),
                Some("90".into()),
                Some("10".into()),
            ],
            vec![
                Some("2".into()),
                Some("1".into()),
                Some("3".into()),
                Some("50".into()),
                None,
            ],
            vec![
                Some("3".into()),
                Some("1".into()),
                Some("6".into()),
                Some("120".into()),
                Some("20".into()),
            ],
            vec![
                Some("4".into()),
                Some("2".into()),
                Some("7".into()),
                Some("7".into()),
                Some("7".into()),
            ],
        ],
    )
    .unwrap();
    write_wide_part(
        &dimension,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "state".into(),
                data_type: "String".into(),
            },
            Column {
                name: "min_value".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "max_value".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "avg_value".into(),
                data_type: "Nullable(UInt64)".into(),
            },
        ],
        &[
            vec![
                Some("1".into()),
                Some("A".into()),
                Some("8".into()),
                Some("40".into()),
                Some("10".into()),
            ],
            vec![
                Some("2".into()),
                Some("A".into()),
                Some("2".into()),
                Some("100".into()),
                Some("30".into()),
            ],
            vec![
                Some("3".into()),
                Some("B".into()),
                Some("4".into()),
                Some("60".into()),
                Some("30".into()),
            ],
            vec![
                Some("4".into()),
                Some("C".into()),
                Some("5".into()),
                Some("5".into()),
                None,
            ],
        ],
    )
    .unwrap();
    let id_schema = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    write_wide_part(
        &dates,
        &id_schema,
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
            vec![Some("4".into())],
        ],
    )
    .unwrap();

    let query = parse_join(
        "SELECT f.group_id, COUNT(*) c1, MIN(f.min_value) min1, MAX(f.max_value) max1, AVG(f.avg_value) avg1, d.state, COUNT(*) c2, MIN(d.min_value) min2, MAX(d.max_value) max2, AVG(d.avg_value) avg2 FROM fact f JOIN dimension d ON f.id=d.id JOIN dates t ON f.id=t.id GROUP BY f.group_id, d.state ORDER BY f.group_id, d.state",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[fact, dimension, dates], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "group_id\tc1\tmin1\tmax1\tavg1\tstate\tc2\tmin2\tmax2\tavg2\n1\t2\t3\t90\t10\tA\t2\t2\t100\t20\n1\t1\t6\t120\t20\tB\t1\t4\t60\t30\n2\t1\t7\t7\t7\tC\t1\t5\t5\t\\N\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn grouped_window_avg_keeps_unprojected_group_keys() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-window-avg-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("sales");
    let items = root.join("items");
    let dates = root.join("dates");
    write_wide_part(
        &sales,
        &[
            Column {
                name: "product_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "date_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "amount".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into()), Some("10".into())],
            vec![Some("1".into()), Some("1".into()), Some("30".into())],
            vec![Some("2".into()), Some("2".into()), Some("60".into())],
            vec![Some("3".into()), Some("3".into()), Some("20".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &items,
        &[
            Column {
                name: "product_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "manufacturer_id".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into())],
            vec![Some("2".into()), Some("1".into())],
            vec![Some("3".into()), Some("2".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &[
            Column {
                name: "date_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "quarter".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into())],
            vec![Some("2".into()), Some("2".into())],
            vec![Some("3".into()), Some("1".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT i.manufacturer_id, SUM(s.amount) total_sales, AVG(SUM(s.amount)) OVER (PARTITION BY i.manufacturer_id) avg_quarterly_sales FROM sales s JOIN items i ON s.product_id=i.product_id JOIN dates d ON s.date_id=d.date_id GROUP BY i.manufacturer_id, d.quarter";
    let query = parse_join(sql).unwrap().unwrap();
    assert_eq!(query.grouped_count.as_ref().unwrap().len(), 1);
    assert_eq!(query.grouped_execution_keys.len(), 2);
    assert_eq!(query.grouped_output.len(), 3);

    let paths = [sales.clone(), items.clone(), dates.clone()];
    let mut output = Vec::new();
    execute_multi_join(&query, &paths, &mut output).unwrap();
    let mut lines = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    lines.sort();
    assert_eq!(
        lines,
        [
            "1\t40\t50",
            "1\t60\t50",
            "2\t20\t20",
            "manufacturer_id\ttotal_sales\tavg_quarterly_sales",
        ]
    );

    let wrapped = "SELECT * FROM (SELECT i.manufacturer_id, SUM(s.amount) total_sales, AVG(SUM(s.amount)) OVER (PARTITION BY i.manufacturer_id) avg_quarterly_sales FROM sales s JOIN items i ON s.product_id=i.product_id JOIN dates d ON s.date_id=d.date_id GROUP BY i.manufacturer_id, d.quarter) tmp1 WHERE CASE WHEN avg_quarterly_sales > 0 THEN ABS(total_sales - avg_quarterly_sales) / avg_quarterly_sales ELSE NULL END > 0.1 ORDER BY avg_quarterly_sales, total_sales, manufacturer_id LIMIT 100";
    let plan = parse_derived_grouped_select_with_schema(wrapped, |table| {
        let columns = match table {
            "sales" => vec![
                ("product_id", "UInt64"),
                ("date_id", "UInt64"),
                ("amount", "UInt64"),
            ],
            "items" => vec![("product_id", "UInt64"), ("manufacturer_id", "UInt64")],
            "dates" => vec![("date_id", "UInt64"), ("quarter", "UInt64")],
            _ => return Err(rusthouse_query::ParseError("unknown fixture table".into())),
        };
        Ok(columns
            .into_iter()
            .map(|(name, data_type)| rusthouse_query::DerivedColumn {
                name: name.into(),
                data_type: data_type.into(),
            })
            .collect())
    })
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_derived_grouped_select(&plan, &paths, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "manufacturer_id\ttotal_sales\tavg_quarterly_sales\n1\t40\t50\n1\t60\t50\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_grouped_avg_combinations_fail_early() {
    for sql in [
        "SELECT a.id, AVG(b.x) AS x FROM a JOIN b ON a.id=b.id GROUP BY a.id",
        "SELECT a.id, AVG(b.x) AS avg_x, AVG(c.y) AS avg_x FROM a JOIN b ON a.id=b.id JOIN c ON b.id=c.id GROUP BY a.id",
        "SELECT a.id, AVG(b.x) FROM a JOIN b ON a.id=b.id LEFT JOIN c ON b.id=c.id GROUP BY a.id",
    ] {
        assert!(parse_join(sql).is_err(), "{sql}");
    }
}
