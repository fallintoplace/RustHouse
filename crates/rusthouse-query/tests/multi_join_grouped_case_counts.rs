use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn grouped_case_counts_evaluate_numeric_expressions_across_join_chains() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-group-case-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("sales");
    let returns = root.join("returns");
    let status = root.join("status");

    write_wide_part(
        &sales,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "region".into(),
                data_type: "String".into(),
            },
            Column {
                name: "sold_sk".into(),
                data_type: "Nullable(UInt64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("east".into()), Some("10".into())],
            vec![Some("2".into()), Some("east".into()), Some("10".into())],
            vec![Some("3".into()), Some("west".into()), Some("10".into())],
            vec![Some("4".into()), Some("west".into()), None],
        ],
    )
    .unwrap();
    write_wide_part(
        &returns,
        &[
            Column {
                name: "sale_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "return_sk".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("101".into()), Some("40".into())],
            vec![Some("2".into()), Some("102".into()), Some("40".into())],
            vec![Some("2".into()), Some("103".into()), Some("50".into())],
            vec![Some("3".into()), Some("104".into()), Some("71".into())],
            vec![Some("4".into()), Some("105".into()), Some("70".into())],
        ],
    )
    .unwrap();
    let status_schema = [Column {
        name: "return_id".into(),
        data_type: "UInt64".into(),
    }];
    write_wide_part(
        &status,
        &status_schema,
        &[
            vec![Some("101".into())],
            vec![Some("102".into())],
            vec![Some("103".into())],
            vec![Some("104".into())],
            vec![Some("105".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT s.region, SUM(CASE WHEN (return_sk - sold_sk <= 30) THEN 1 ELSE 0 END) AS within_30, SUM(CASE WHEN (return_sk - sold_sk > 30) AND (return_sk - sold_sk <= 60) THEN 1 ELSE 0 END) AS within_60, SUM(CASE WHEN (return_sk - sold_sk > 60) THEN 1 ELSE 0 END) AS over_60 FROM sales s JOIN returns r ON s.id = r.sale_id JOIN status t ON r.id = t.return_id GROUP BY s.region ORDER BY s.region";
    let query = parse_join(sql).unwrap().unwrap();
    assert_eq!(query.grouped_case_counts.len(), 3);
    let mut output = Vec::new();
    execute_multi_join(&query, &[sales, returns, status], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "region\twithin_30\twithin_60\tover_60\neast\t2\t1\t0\nwest\t0\t0\t1\n"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn grouped_case_sums_preserve_nulls_and_order_by_metric_aliases() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-group-case-sum-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let dates = root.join("date_dim");
    let sales = root.join("store_sales");
    let stores = root.join("store");

    write_wide_part(
        &dates,
        &[
            Column {
                name: "d_date_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "d_day_name".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("101".into()), Some("Sunday".into())],
            vec![Some("102".into()), Some("Monday".into())],
            vec![Some("103".into()), Some("Tuesday".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &sales,
        &[
            Column {
                name: "ss_sold_date_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "ss_store_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "ss_sales_price".into(),
                data_type: "Float64".into(),
            },
        ],
        &[
            vec![Some("101".into()), Some("1".into()), Some("5.5".into())],
            vec![Some("102".into()), Some("1".into()), Some("7.25".into())],
            vec![Some("103".into()), Some("1".into()), Some("2".into())],
            vec![Some("101".into()), Some("2".into()), Some("10".into())],
            vec![Some("103".into()), Some("2".into()), Some("4".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &stores,
        &[
            Column {
                name: "s_store_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "s_store_name".into(),
                data_type: "String".into(),
            },
            Column {
                name: "s_store_id".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("A".into()), Some("1".into())],
            vec![Some("2".into()), Some("B".into()), Some("2".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT s_store_name, s_store_id, SUM(CASE WHEN (d_day_name='Sunday') THEN ss_sales_price ELSE NULL END) sun_sales, SUM(CASE WHEN (d_day_name='Monday') THEN ss_sales_price ELSE NULL END) mon_sales, SUM(CASE WHEN (d_day_name='Tuesday') THEN ss_sales_price ELSE NULL END) tue_sales FROM date_dim d JOIN store_sales ss ON d.d_date_sk = ss.ss_sold_date_sk JOIN store s ON s.s_store_sk = ss.ss_store_sk GROUP BY s_store_name, s_store_id ORDER BY s_store_name, s_store_id, sun_sales, mon_sales, tue_sales";
    let query = parse_join(sql).unwrap().unwrap();
    assert_eq!(query.grouped_case_counts.len(), 3);
    let mut output = Vec::new();
    execute_multi_join(&query, &[dates, sales, stores], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "s_store_name\ts_store_id\tsun_sales\tmon_sales\ttue_sales\nA\t1\t5.5\t7.25\t2\nB\t2\t10\t\\N\t4\n"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn grouped_case_null_counts_include_outer_join_rows_and_total_count() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-group-case-null-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("sales");
    let promotion = root.join("promotion");
    write_wide_part(
        &sales,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "region".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("east".into())],
            vec![Some("2".into()), Some("east".into())],
            vec![Some("3".into()), Some("west".into())],
            vec![Some("4".into()), Some("west".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &promotion,
        &[
            Column {
                name: "sale_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "promo_sk".into(),
                data_type: "Nullable(UInt64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("10".into())],
            vec![Some("1".into()), Some("11".into())],
            vec![Some("2".into()), None],
            vec![Some("4".into()), Some("20".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT s.region, SUM(CASE WHEN p.promo_sk IS NULL THEN 1 ELSE 0 END) AS no_promo, SUM(CASE WHEN p.promo_sk IS NOT NULL THEN 1 ELSE 0 END) AS promo, COUNT(*) AS total_cnt FROM sales s LEFT JOIN promotion p ON sale_id = id GROUP BY s.region ORDER BY s.region";
    let query = parse_join(sql).unwrap().unwrap();
    assert_eq!(query.grouped_case_counts.len(), 3);
    assert!(query.grouped_count.is_some());
    assert!(!query.grouped_keys_only);
    let mut output = Vec::new();
    execute_multi_join(&query, &[sales, promotion], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "region\tno_promo\tpromo\ttotal_cnt\neast\t1\t2\t3\nwest\t1\t1\t2\n"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn grouped_case_null_counts_preserve_rows_across_left_join_chain() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-group-case-left-chain-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("sales");
    let details = root.join("details");
    let promotion = root.join("promotion");
    let returns = root.join("returns");

    write_wide_part(
        &sales,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "region".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("east".into())],
            vec![Some("2".into()), Some("east".into())],
            vec![Some("3".into()), Some("west".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &details,
        &[Column {
            name: "sale_id".into(),
            data_type: "UInt64".into(),
        }],
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &promotion,
        &[
            Column {
                name: "sale_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "promo_sk".into(),
                data_type: "Nullable(UInt64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("10".into())],
            vec![Some("1".into()), Some("11".into())],
            vec![Some("3".into()), None],
        ],
    )
    .unwrap();
    write_wide_part(
        &returns,
        &[
            Column {
                name: "sale_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "return_id".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("20".into())],
            vec![Some("3".into()), Some("30".into())],
            vec![Some("3".into()), Some("31".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT s.region, SUM(CASE WHEN p.promo_sk IS NULL THEN 1 ELSE 0 END) AS no_promo, SUM(CASE WHEN p.promo_sk IS NOT NULL THEN 1 ELSE 0 END) AS promo, COUNT(*) AS total_cnt FROM sales s JOIN details d ON sale_id = id LEFT JOIN promotion p ON d.sale_id = p.sale_id LEFT JOIN returns r ON d.sale_id = r.sale_id GROUP BY s.region ORDER BY s.region";
    let query = parse_join(sql).unwrap().unwrap();
    assert_eq!(query.steps.len(), 2);
    assert_eq!(query.grouped_case_counts.len(), 3);
    let mut output = Vec::new();
    execute_multi_join(&query, &[sales, details, promotion, returns], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "region\tno_promo\tpromo\ttotal_cnt\neast\t1\t2\t3\nwest\t2\t0\t2\n"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn grouped_case_counts_read_simple_substring_derived_columns() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-derived-substring-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("sales");
    let warehouses = root.join("warehouse");
    let dates = root.join("dates");

    write_wide_part(
        &sales,
        &[
            Column {
                name: "warehouse_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "day_id".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("101".into())],
            vec![Some("1".into()), Some("102".into())],
            vec![Some("2".into()), Some("101".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &warehouses,
        &[
            Column {
                name: "w_warehouse_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "w_warehouse_name".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("Alpha North".into())],
            vec![Some("2".into()), Some("Beta South".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &[
            Column {
                name: "d_date_sk".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "d_day_name".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("101".into()), Some("Sunday".into())],
            vec![Some("102".into()), Some("Monday".into())],
        ],
    )
    .unwrap();

    let sql = "SELECT w_substr, d_day_name, SUM(CASE WHEN (d_day_name='Sunday') THEN 1 ELSE 0 END) AS sunday_rows FROM sales f JOIN (SELECT SUBSTRING(w_warehouse_name,1,3) w_substr, * FROM warehouse) wh ON f.warehouse_id = wh.w_warehouse_sk JOIN dates d ON f.day_id = d.d_date_sk GROUP BY w_substr, d_day_name ORDER BY w_substr, d_day_name";
    let query = parse_join(sql).unwrap().unwrap();
    assert_eq!(query.virtual_columns.len(), 1);
    assert_eq!(query.virtual_columns[0].name, "w_substr");
    let mut output = Vec::new();
    execute_multi_join(&query, &[sales, warehouses, dates], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "w_substr\td_day_name\tsunday_rows\nAlp\tMonday\t0\nAlp\tSunday\t1\nBet\tSunday\t1\n"
    );

    fs::remove_dir_all(root).unwrap();
}
