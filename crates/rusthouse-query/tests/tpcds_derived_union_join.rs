use rusthouse_query::{
    DerivedColumn, execute_derived_union_join, has_derived_union,
    parse_derived_union_join_with_schema,
};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const Q71: &str = "SELECT i_brand_id brand_id, i_brand brand, t_hour, t_minute, sum(ext_price) ext_price FROM item, (SELECT ws_ext_sales_price AS ext_price, ws_sold_date_sk AS sold_date_sk, ws_item_sk AS sold_item_sk, ws_sold_time_sk AS time_sk FROM web_sales, date_dim WHERE d_date_sk = ws_sold_date_sk AND d_moy = 11 AND d_year = 1999 UNION ALL SELECT cs_ext_sales_price AS ext_price, cs_sold_date_sk AS sold_date_sk, cs_item_sk AS sold_item_sk, cs_sold_time_sk AS time_sk FROM catalog_sales, date_dim WHERE d_date_sk = cs_sold_date_sk AND d_moy = 11 AND d_year = 1999 UNION ALL SELECT ss_ext_sales_price AS ext_price, ss_sold_date_sk AS sold_date_sk, ss_item_sk AS sold_item_sk, ss_sold_time_sk AS time_sk FROM store_sales, date_dim WHERE d_date_sk = ss_sold_date_sk AND d_moy = 11 AND d_year = 1999) tmp, time_dim WHERE sold_item_sk = i_item_sk AND i_manager_id = 1 AND time_sk = t_time_sk AND (t_meal_time = 'breakfast' OR t_meal_time = 'dinner') GROUP BY i_brand, i_brand_id, t_hour, t_minute ORDER BY ext_price DESC NULLS FIRST, i_brand_id NULLS FIRST, t_hour NULLS FIRST, t_minute NULLS FIRST";

fn schema(table: &str) -> Result<Vec<DerivedColumn>, rusthouse_query::ParseError> {
    let columns: &[(&str, &str)] = match table {
        "item" => &[
            ("i_item_sk", "UInt64"),
            ("i_brand_id", "UInt64"),
            ("i_brand", "String"),
            ("i_manager_id", "UInt64"),
        ],
        "time_dim" => &[
            ("t_time_sk", "UInt64"),
            ("t_hour", "UInt64"),
            ("t_minute", "UInt64"),
            ("t_meal_time", "String"),
        ],
        "date_dim" => &[
            ("d_date_sk", "UInt64"),
            ("d_moy", "UInt64"),
            ("d_year", "UInt64"),
        ],
        "web_sales" => &[
            ("ws_ext_sales_price", "UInt64"),
            ("ws_sold_date_sk", "UInt64"),
            ("ws_item_sk", "UInt64"),
            ("ws_sold_time_sk", "UInt64"),
        ],
        "catalog_sales" => &[
            ("cs_ext_sales_price", "UInt64"),
            ("cs_sold_date_sk", "UInt64"),
            ("cs_item_sk", "UInt64"),
            ("cs_sold_time_sk", "UInt64"),
        ],
        "store_sales" => &[
            ("ss_ext_sales_price", "UInt64"),
            ("ss_sold_date_sk", "UInt64"),
            ("ss_item_sk", "UInt64"),
            ("ss_sold_time_sk", "UInt64"),
        ],
        _ => return Err(rusthouse_query::ParseError("unknown table".into())),
    };
    Ok(columns
        .iter()
        .map(|(name, data_type)| DerivedColumn {
            name: (*name).into(),
            data_type: (*data_type).into(),
        })
        .collect())
}

fn write_part(path: &std::path::Path, columns: &[(&str, &str)], rows: &[&[&str]]) {
    let schema = columns
        .iter()
        .map(|(name, data_type)| Column {
            name: (*name).into(),
            data_type: (*data_type).into(),
        })
        .collect::<Vec<_>>();
    let rows = rows
        .iter()
        .map(|row| row.iter().map(|value| Some((*value).into())).collect())
        .collect::<Vec<Vec<Option<String>>>>();
    write_wide_part(path, &schema, &rows).unwrap();
}

#[test]
fn unions_three_projected_sales_branches_before_grouped_join() {
    assert!(has_derived_union(Q71).unwrap());
    let plan = parse_derived_union_join_with_schema(Q71, schema)
        .unwrap()
        .unwrap();
    assert_eq!(plan.branches().len(), 3);
    assert_eq!(
        plan.columns()
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["ext_price", "sold_date_sk", "sold_item_sk", "time_sk"]
    );

    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-derived-union-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let item = root.join("item");
    let time = root.join("time_dim");
    let date = root.join("date_dim");
    let web = root.join("web_sales");
    let catalog = root.join("catalog_sales");
    let store = root.join("store_sales");
    write_part(
        &item,
        &[
            ("i_item_sk", "UInt64"),
            ("i_brand_id", "UInt64"),
            ("i_brand", "String"),
            ("i_manager_id", "UInt64"),
        ],
        &[&["1", "7", "Brand A", "1"], &["2", "7", "Brand A", "1"]],
    );
    write_part(
        &time,
        &[
            ("t_time_sk", "UInt64"),
            ("t_hour", "UInt64"),
            ("t_minute", "UInt64"),
            ("t_meal_time", "String"),
        ],
        &[
            &["1", "8", "15", "breakfast"],
            &["2", "19", "0", "dinner"],
            &["3", "12", "0", "lunch"],
        ],
    );
    write_part(
        &date,
        &[
            ("d_date_sk", "UInt64"),
            ("d_moy", "UInt64"),
            ("d_year", "UInt64"),
        ],
        &[&["1", "11", "1999"], &["2", "10", "1999"]],
    );
    write_part(
        &web,
        &[
            ("ws_ext_sales_price", "UInt64"),
            ("ws_sold_date_sk", "UInt64"),
            ("ws_item_sk", "UInt64"),
            ("ws_sold_time_sk", "UInt64"),
        ],
        &[&["30", "1", "1", "2"], &["10", "1", "1", "1"]],
    );
    write_part(
        &catalog,
        &[
            ("cs_ext_sales_price", "UInt64"),
            ("cs_sold_date_sk", "UInt64"),
            ("cs_item_sk", "UInt64"),
            ("cs_sold_time_sk", "UInt64"),
        ],
        &[&["99", "1", "1", "3"]],
    );
    write_part(
        &store,
        &[
            ("ss_ext_sales_price", "UInt64"),
            ("ss_sold_date_sk", "UInt64"),
            ("ss_item_sk", "UInt64"),
            ("ss_sold_time_sk", "UInt64"),
        ],
        &[
            &["10", "1", "1", "1"],
            &["20", "1", "2", "2"],
            &["500", "2", "1", "1"],
        ],
    );
    let by_table = |table: &str| match table {
        "item" => item.clone(),
        "time_dim" => time.clone(),
        "date_dim" => date.clone(),
        "web_sales" => web.clone(),
        "catalog_sales" => catalog.clone(),
        "store_sales" => store.clone(),
        _ => panic!("unexpected table {table}"),
    };
    let branch_paths = plan
        .branches()
        .iter()
        .map(|branch| {
            [&branch.left, &branch.right]
                .into_iter()
                .chain(branch.steps.iter().map(|step| &step.table))
                .map(|table| by_table(table))
                .collect::<Vec<PathBuf>>()
        })
        .collect::<Vec<_>>();
    let outer = plan.outer();
    let outer_paths = [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table))
        .map(|table| {
            if table == plan.table() {
                PathBuf::new()
            } else {
                by_table(table)
            }
        })
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    execute_derived_union_join(&plan, &branch_paths, &outer_paths, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "brand_id\tbrand\tt_hour\tt_minute\text_price\n7\tBrand A\t19\t0\t50\n7\tBrand A\t8\t15\t20\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_derived_union_branches_with_different_output_widths() {
    let sql = "SELECT x FROM item, (SELECT ws_item_sk AS x FROM web_sales, date_dim WHERE ws_sold_date_sk = d_date_sk UNION ALL SELECT cs_ext_sales_price AS y, cs_item_sk AS z FROM catalog_sales, date_dim WHERE cs_sold_date_sk = d_date_sk) tmp";
    assert!(
        parse_derived_union_join_with_schema(sql, schema)
            .unwrap()
            .is_none()
    );
}

#[test]
fn uses_first_branch_names_when_later_union_aliases_differ() {
    let sql = "SELECT i_brand_id, SUM(ext_price) AS amount FROM item, (SELECT ws_ext_sales_price AS ext_price, ws_item_sk AS sold_item_sk FROM web_sales, date_dim WHERE ws_sold_date_sk = d_date_sk UNION ALL SELECT cs_ext_sales_price AS price, cs_item_sk AS item_key FROM catalog_sales, date_dim WHERE cs_sold_date_sk = d_date_sk) tmp WHERE sold_item_sk = i_item_sk GROUP BY i_brand_id";
    let plan = parse_derived_union_join_with_schema(sql, schema)
        .unwrap()
        .unwrap();
    assert_eq!(
        plan.columns()
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["ext_price", "sold_item_sk"]
    );
}

#[test]
fn does_not_detect_union_distinct_as_a_union_all_plan() {
    let sql = "SELECT x FROM item, (SELECT ws_item_sk AS x FROM web_sales, date_dim WHERE ws_sold_date_sk = d_date_sk UNION SELECT cs_item_sk AS x FROM catalog_sales, date_dim WHERE cs_sold_date_sk = d_date_sk) tmp";
    assert!(!has_derived_union(sql).unwrap());
    assert!(
        parse_derived_union_join_with_schema(sql, schema)
            .unwrap()
            .is_none()
    );
}

#[test]
fn rejects_derived_union_branches_with_different_types() {
    let sql = "SELECT SUM(x) FROM (SELECT ws_item_sk AS x FROM web_sales, date_dim WHERE ws_sold_date_sk = d_date_sk UNION ALL SELECT cs_item_sk AS x FROM catalog_sales, date_dim WHERE cs_sold_date_sk = d_date_sk) tmp";
    let mismatched_schema =
        |table: &str| -> Result<Vec<DerivedColumn>, rusthouse_query::ParseError> {
            let columns: &[(&str, &str)] = match table {
                "web_sales" => &[("ws_sold_date_sk", "UInt64"), ("ws_item_sk", "UInt64")],
                "catalog_sales" => &[("cs_sold_date_sk", "UInt64"), ("cs_item_sk", "String")],
                "date_dim" => &[("d_date_sk", "UInt64")],
                _ => return Err(rusthouse_query::ParseError("unknown table".into())),
            };
            Ok(columns
                .iter()
                .map(|(name, data_type)| DerivedColumn {
                    name: (*name).into(),
                    data_type: (*data_type).into(),
                })
                .collect())
        };
    assert!(
        parse_derived_union_join_with_schema(sql, mismatched_schema)
            .unwrap()
            .is_none()
    );
}
