use rusthouse_query::{execute_correlated_avg_with_paths, parse_join_correlated_avg_with_schema};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn joined_correlated_avg_filters_before_scalar_sum() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-correlated-avg-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("catalog_sales");
    let item = root.join("item");
    let dates = root.join("date_dim");
    write_wide_part(
        &sales,
        &[
            Column {
                name: "cs_item_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "cs_sold_date_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "cs_ext_discount_amt".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into()), Some("100".into())],
            vec![Some("1".into()), Some("2".into()), Some("200".into())],
            vec![Some("1".into()), Some("3".into()), Some("500".into())],
            vec![Some("2".into()), Some("1".into()), Some("10".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &item,
        &[
            Column {
                name: "i_item_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "i_manufact_id".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[vec![Some("1".into()), Some("977".into())]],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &[Column {
            name: "d_date_sk".into(),
            data_type: "UInt64".into(),
        }],
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT SUM(cs_ext_discount_amt) AS total FROM catalog_sales, item, date_dim WHERE i_manufact_id = 977 AND i_item_sk = cs_item_sk AND d_date_sk BETWEEN 1 AND 2 AND d_date_sk = cs_sold_date_sk AND cs_ext_discount_amt > (SELECT 1.3 * AVG(cs_ext_discount_amt) FROM catalog_sales, date_dim WHERE cs_item_sk = i_item_sk AND d_date_sk BETWEEN 1 AND 2 AND d_date_sk = cs_sold_date_sk)";
    let schema = |table: &str| -> Result<Vec<String>, rusthouse_query::ParseError> {
        let columns = match table {
            "catalog_sales" => vec!["cs_item_sk", "cs_sold_date_sk", "cs_ext_discount_amt"],
            "item" => vec!["i_item_sk", "i_manufact_id"],
            "date_dim" => vec!["d_date_sk"],
            _ => return Err(rusthouse_query::ParseError("unknown table".into())),
        };
        Ok(columns.into_iter().map(str::to_owned).collect())
    };
    let plan = parse_join_correlated_avg_with_schema(sql, schema)
        .unwrap()
        .unwrap();
    let outer = plan.outer();
    let table_path = |table: &str| -> PathBuf {
        match table {
            "catalog_sales" => sales.clone(),
            "item" => item.clone(),
            "date_dim" => dates.clone(),
            _ => panic!("unexpected outer table {table}"),
        }
    };
    let outer_paths = [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table))
        .map(|table| table_path(table))
        .collect::<Vec<_>>();
    let inner_paths = plan
        .inner_tables()
        .iter()
        .map(|table| table_path(table))
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    execute_correlated_avg_with_paths(&plan, &outer_paths, &inner_paths, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "total\n200\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn joined_correlated_avg_retains_unprojected_filter_value_when_ordered() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-correlated-avg-projection-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("catalog_sales");
    let item = root.join("item");
    let dates = root.join("date_dim");
    write_wide_part(
        &sales,
        &[
            Column {
                name: "cs_item_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "cs_sold_date_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "cs_ext_discount_amt".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into()), Some("100".into())],
            vec![Some("1".into()), Some("2".into()), Some("200".into())],
            vec![Some("1".into()), Some("3".into()), Some("500".into())],
            vec![Some("2".into()), Some("1".into()), Some("10".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &item,
        &[
            Column {
                name: "i_item_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "i_manufact_id".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[vec![Some("1".into()), Some("977".into())]],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &[Column {
            name: "d_date_sk".into(),
            data_type: "UInt64".into(),
        }],
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT i_item_sk, d_date_sk FROM catalog_sales, item, date_dim WHERE i_manufact_id = 977 AND i_item_sk = cs_item_sk AND d_date_sk BETWEEN 1 AND 2 AND d_date_sk = cs_sold_date_sk AND cs_ext_discount_amt > (SELECT AVG(cs_ext_discount_amt) FROM catalog_sales, date_dim WHERE cs_item_sk = i_item_sk AND d_date_sk BETWEEN 1 AND 2 AND d_date_sk = cs_sold_date_sk) ORDER BY d_date_sk";
    let schema = |table: &str| -> Result<Vec<String>, rusthouse_query::ParseError> {
        let columns = match table {
            "catalog_sales" => vec!["cs_item_sk", "cs_sold_date_sk", "cs_ext_discount_amt"],
            "item" => vec!["i_item_sk", "i_manufact_id"],
            "date_dim" => vec!["d_date_sk"],
            _ => return Err(rusthouse_query::ParseError("unknown table".into())),
        };
        Ok(columns.into_iter().map(str::to_owned).collect())
    };
    let plan = parse_join_correlated_avg_with_schema(sql, schema)
        .unwrap()
        .unwrap();
    let outer = plan.outer();
    let table_path = |table: &str| -> PathBuf {
        match table {
            "catalog_sales" => sales.clone(),
            "item" => item.clone(),
            "date_dim" => dates.clone(),
            _ => panic!("unexpected outer table {table}"),
        }
    };
    let outer_paths = [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table))
        .map(|table| table_path(table))
        .collect::<Vec<_>>();
    let inner_paths = plan
        .inner_tables()
        .iter()
        .map(|table| table_path(table))
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    execute_correlated_avg_with_paths(&plan, &outer_paths, &inner_paths, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "i_item_sk\td_date_sk\n1\t2\n"
    );

    fs::remove_dir_all(root).unwrap();
}
