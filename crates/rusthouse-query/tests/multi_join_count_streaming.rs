use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn scalar_count_order_scans_a_bounded_chain_above_the_buffered_row_limit() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-multi-join-count-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let dates = root.join("dates");
    let items = root.join("items");
    let key = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    let rows = vec![vec![Some("1".into())]; 32_769];
    write_wide_part(&fact, &key, &rows).unwrap();
    write_wide_part(&dates, &key, &[vec![Some("1".into())]]).unwrap();
    write_wide_part(&items, &key, &[vec![Some("1".into())]]).unwrap();

    let query = parse_join(
        "SELECT COUNT(*) FROM fact f JOIN dates d ON f.id = d.id JOIN items i ON d.id = i.id ORDER BY COUNT(*) LIMIT 100",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[fact, dates, items], &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "count_star()\n32769\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn chained_scalar_sum_applies_in_filters_while_streaming() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-multi-join-sum-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let categories = root.join("categories");
    let flags = root.join("flags");
    write_wide_part(
        &fact,
        &[
            Column {
                name: "category_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "flag_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "amount".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("10".into()), Some("5".into())],
            vec![Some("2".into()), Some("20".into()), Some("7".into())],
            vec![Some("1".into()), Some("20".into()), Some("11".into())],
        ],
    )
    .unwrap();
    let key = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    write_wide_part(
        &categories,
        &key,
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();
    write_wide_part(
        &flags,
        &[
            key[0].clone(),
            Column {
                name: "kind".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("10".into()), Some("keep".into())],
            vec![Some("20".into()), Some("skip".into())],
        ],
    )
    .unwrap();

    let query = parse_join(
        "SELECT SUM(f.amount) AS total FROM fact f JOIN categories c ON f.category_id = c.id JOIN flags g ON f.flag_id = g.id WHERE g.kind IN ('keep')",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[fact, categories, flags], &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "total\n5\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn chained_scalar_aggregates_keep_independent_null_semantics() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-multi-join-aggregates-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let dimension = root.join("dimension");
    write_wide_part(
        &fact,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "amount".into(),
                data_type: "Nullable(Float64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("5.0".into())],
            vec![Some("1".into()), None],
            vec![Some("1".into()), Some("5.0".into())],
            vec![Some("2".into()), Some("8.0".into())],
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
                name: "keep".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("yes".into())],
            vec![Some("2".into()), Some("no".into())],
        ],
    )
    .unwrap();

    let query = parse_join(
        "SELECT AVG(f.amount) AS avg_amount, SUM(f.amount) total, COUNT(*) rows, COUNT(f.amount) present, COUNT(DISTINCT f.amount) unique_amounts FROM fact f JOIN dimension d ON f.id = d.id WHERE d.keep = 'yes'",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[fact, dimension], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "avg_amount\ttotal\trows\tpresent\tunique_amounts\n5\t10\t3\t2\t1\n"
    );

    fs::remove_dir_all(root).unwrap();
}
