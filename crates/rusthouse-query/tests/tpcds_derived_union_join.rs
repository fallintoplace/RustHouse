use rusthouse_query::{
    DerivedColumn, execute_derived_union_join, execute_derived_union_select, has_derived_union,
    parse_cte_union_select_with_schema, parse_derived_union_join_with_schema,
    parse_derived_union_select_with_schema, parse_join,
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
            ("i_manufact_id", "UInt64"),
            ("i_category", "String"),
            ("i_item_id", "String"),
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
            ("d_date", "Date32"),
        ],
        "customer_address" => &[("ca_address_sk", "UInt64"), ("ca_gmt_offset", "Float64")],
        "web_sales" => &[
            ("ws_ext_sales_price", "UInt64"),
            ("ws_sold_date_sk", "UInt64"),
            ("ws_item_sk", "UInt64"),
            ("ws_sold_time_sk", "UInt64"),
            ("ws_sales_price", "UInt64"),
            ("ws_bill_addr_sk", "UInt64"),
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
            ("ss_sales_price", "UInt64"),
            ("ss_addr_sk", "UInt64"),
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
fn unions_text_literal_labels_with_projected_source_columns() {
    let sql = "SELECT channel, COUNT(*) AS sales_count, SUM(ext_price) AS sales_amount FROM (SELECT 'store' AS channel, ss_ext_sales_price AS ext_price FROM store_sales, date_dim WHERE ss_sold_date_sk = d_date_sk UNION ALL SELECT 'web' AS channel, ws_ext_sales_price AS ext_price FROM web_sales, date_dim WHERE ws_sold_date_sk = d_date_sk) sales GROUP BY channel";
    let branch = parse_join("SELECT 'store' AS channel, ss_ext_sales_price AS ext_price FROM store_sales JOIN date_dim ON ss_sold_date_sk = d_date_sk")
        .unwrap()
        .unwrap();
    assert_eq!(branch.scalar_projections.len(), 1);
    assert!(has_derived_union(sql).unwrap());
    let plan = parse_derived_union_select_with_schema(sql, schema)
        .unwrap()
        .unwrap();

    assert_eq!(plan.branches().len(), 2);
    assert_eq!(plan.columns()[0].name, "channel");
    assert_eq!(plan.columns()[0].data_type, "String");
    assert_eq!(plan.columns()[1].name, "ext_price");
    assert!(
        plan.branches()[0].scalar_projections[0]
            .expression
            .eq(&rusthouse_query::ScalarExpr::TextLiteral("store".into()))
    );
}

#[test]
fn executes_single_select_over_text_labeled_union_branches() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-derived-union-select-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let dates = root.join("date_dim");
    let store = root.join("store_sales");
    let web = root.join("web_sales");
    write_part(&dates, &[("d_date_sk", "UInt64")], &[&["1"], &["2"]]);
    write_part(
        &store,
        &[
            ("ss_sold_date_sk", "UInt64"),
            ("ss_ext_sales_price", "UInt64"),
        ],
        &[&["1", "10"], &["1", "5"], &["2", "7"]],
    );
    write_part(
        &web,
        &[
            ("ws_sold_date_sk", "UInt64"),
            ("ws_ext_sales_price", "UInt64"),
        ],
        &[&["1", "20"], &["2", "3"]],
    );

    let sql = "SELECT channel, COUNT(*) AS sales_count, SUM(ext_price) AS sales_amount FROM (SELECT 'store' AS channel, ss_ext_sales_price AS ext_price FROM store_sales, date_dim WHERE ss_sold_date_sk = d_date_sk UNION ALL SELECT 'web' AS channel, ws_ext_sales_price AS ext_price FROM web_sales, date_dim WHERE ws_sold_date_sk = d_date_sk) sales GROUP BY channel ORDER BY channel";
    let plan = parse_derived_union_select_with_schema(sql, schema)
        .unwrap()
        .unwrap();
    let branch_paths = vec![vec![store, dates.clone()], vec![web, dates]];
    let mut output = Vec::new();
    execute_derived_union_select(&plan, &branch_paths, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "channel\tsales_count\tsales_amount\nstore\t3\t22\nweb\t2\t23\n"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn executes_union_of_grouped_case_branches_with_literal_labels() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-grouped-union-select-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let dates = root.join("date_dim");
    let item = root.join("item");
    let store = root.join("store_sales");
    let web = root.join("web_sales");
    write_part(&dates, &[("d_date_sk", "UInt64")], &[&["1"], &["2"]]);
    write_part(&item, &[("i_item_sk", "UInt64")], &[&["1"]]);
    write_part(
        &store,
        &[
            ("ss_sold_date_sk", "UInt64"),
            ("ss_item_sk", "UInt64"),
            ("ss_ext_sales_price", "UInt64"),
        ],
        &[&["1", "1", "10"], &["1", "1", "5"], &["2", "1", "7"]],
    );
    write_part(
        &web,
        &[
            ("ws_sold_date_sk", "UInt64"),
            ("ws_item_sk", "UInt64"),
            ("ws_ext_sales_price", "UInt64"),
        ],
        &[&["1", "1", "20"], &["2", "1", "3"]],
    );

    let sql = "SELECT channel, SUM(amount) AS sales_amount FROM (SELECT 'store' AS channel, d_date_sk AS day, SUM(CASE WHEN ss_ext_sales_price > 0 THEN ss_ext_sales_price ELSE 0 END) AS amount FROM store_sales, item, date_dim WHERE ss_item_sk = i_item_sk AND ss_sold_date_sk = d_date_sk GROUP BY d_date_sk UNION ALL SELECT 'web' AS channel, d_date_sk AS day, SUM(CASE WHEN ws_ext_sales_price > 0 THEN ws_ext_sales_price ELSE 0 END) AS amount FROM web_sales, item, date_dim WHERE ws_item_sk = i_item_sk AND ws_sold_date_sk = d_date_sk GROUP BY d_date_sk) sales GROUP BY channel ORDER BY channel";
    let plan = parse_derived_union_select_with_schema(sql, schema)
        .unwrap()
        .unwrap();
    let branch_paths = vec![
        vec![store, item.clone(), dates.clone()],
        vec![web, item, dates],
    ];
    let mut output = Vec::new();
    execute_derived_union_select(&plan, &branch_paths, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "channel\tsales_amount\nstore\t22\nweb\t23\n"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn executes_grouped_sum_ctes_combined_by_union_all() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-grouped-sum-cte-union-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let dates = root.join("date_dim");
    let item = root.join("item");
    let store = root.join("store_sales");
    let web = root.join("web_sales");
    write_part(&dates, &[("d_date_sk", "UInt64")], &[&["1"], &["2"]]);
    write_part(
        &item,
        &[
            ("i_item_sk", "UInt64"),
            ("i_manufact_id", "UInt64"),
            ("i_category", "String"),
            ("i_item_id", "String"),
        ],
        &[
            &["1", "10", "Electronics", "ITEM-10"],
            &["2", "20", "Books", "ITEM-20"],
        ],
    );
    write_part(
        &store,
        &[
            ("ss_sold_date_sk", "UInt64"),
            ("ss_item_sk", "UInt64"),
            ("ss_ext_sales_price", "UInt64"),
        ],
        &[&["1", "1", "10"], &["1", "1", "5"], &["2", "2", "7"]],
    );
    write_part(
        &web,
        &[
            ("ws_sold_date_sk", "UInt64"),
            ("ws_item_sk", "UInt64"),
            ("ws_ext_sales_price", "UInt64"),
        ],
        &[&["1", "1", "20"], &["2", "2", "3"]],
    );

    let sql = "WITH store_totals AS (SELECT i_item_id, SUM(ss_ext_sales_price) AS total_sales FROM store_sales, item, date_dim WHERE i_item_id IN (SELECT i_item_id FROM item WHERE i_category IN ('Electronics')) AND ss_item_sk = i_item_sk AND ss_sold_date_sk = d_date_sk GROUP BY i_item_id), web_totals AS (SELECT i_item_id, SUM(ws_ext_sales_price) AS total_sales FROM web_sales, item, date_dim WHERE i_item_id IN (SELECT i_item_id FROM item WHERE i_category IN ('Electronics')) AND ws_item_sk = i_item_sk AND ws_sold_date_sk = d_date_sk GROUP BY i_item_id) SELECT i_item_id, SUM(total_sales) AS total_sales FROM (SELECT * FROM store_totals UNION ALL SELECT * FROM web_totals) tmp GROUP BY i_item_id ORDER BY i_item_id";
    let plan = parse_cte_union_select_with_schema(sql, schema)
        .unwrap()
        .unwrap();
    assert_eq!(plan.branches().len(), 2);
    assert_eq!(plan.columns()[0].name, "i_item_id");
    assert_eq!(plan.columns()[1].name, "total_sales");
    assert_eq!(plan.columns()[1].data_type, "Nullable(UInt128)");

    let mut output = Vec::new();
    execute_derived_union_select(
        &plan,
        &[
            vec![store, item.clone(), dates.clone()],
            vec![web, item, dates],
        ],
        &mut output,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "i_item_id\ttotal_sales\nITEM-10\t35\n"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_cte_union_that_does_not_select_a_cte_star() {
    let sql = "WITH store_totals AS (SELECT i_manufact_id, SUM(ss_ext_sales_price) AS total_sales FROM store_sales, item, date_dim WHERE ss_item_sk = i_item_sk AND ss_sold_date_sk = d_date_sk GROUP BY i_manufact_id) SELECT i_manufact_id, SUM(total_sales) AS total_sales FROM (SELECT i_manufact_id, total_sales FROM store_totals UNION ALL SELECT i_manufact_id, total_sales FROM store_totals) tmp GROUP BY i_manufact_id";
    assert!(
        parse_cte_union_select_with_schema(sql, schema)
            .unwrap()
            .is_none()
    );
}

#[test]
fn parses_tpcds_grouped_cte_union_with_in_subqueries() {
    let store = "SELECT i_manufact_id, SUM(ss_ext_sales_price) AS total_sales FROM store_sales, date_dim, customer_address, item WHERE i_manufact_id IN (SELECT i_manufact_id FROM item WHERE i_category IN ('Electronics')) AND ss_item_sk = i_item_sk AND ss_sold_date_sk = d_date_sk AND d_year = 1998 AND d_moy = 5 AND ss_addr_sk = ca_address_sk AND ca_gmt_offset = -5 GROUP BY i_manufact_id";
    let web = "SELECT i_manufact_id, SUM(ws_ext_sales_price) AS total_sales FROM web_sales, date_dim, customer_address, item WHERE i_manufact_id IN (SELECT i_manufact_id FROM item WHERE i_category IN ('Electronics')) AND ws_item_sk = i_item_sk AND ws_sold_date_sk = d_date_sk AND d_year = 1998 AND d_moy = 5 AND ws_bill_addr_sk = ca_address_sk AND ca_gmt_offset = -5 GROUP BY i_manufact_id";
    let sql = format!(
        "WITH ss AS ({store}), ws AS ({web}) SELECT i_manufact_id, SUM(total_sales) AS total_sales FROM (SELECT * FROM ss UNION ALL SELECT * FROM ws) tmp GROUP BY i_manufact_id ORDER BY total_sales"
    );
    let parsed_store = rusthouse_query::parse_join_with_schema(store, |table| {
        schema(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
    });
    assert!(
        parsed_store.as_ref().is_ok_and(Option::is_some),
        "store CTE branch parse: {parsed_store:?}"
    );
    let plan = parse_cte_union_select_with_schema(&sql, schema)
        .unwrap()
        .expect("TPC-DS grouped CTE UNION shape should be supported");
    assert_eq!(plan.branches().len(), 2);
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
