use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn grouped_inner_join_chain_emits_distinct_keys_without_count() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-group-only-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let dimension = root.join("dimension");
    let dates = root.join("dates");
    let fact_schema = [
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "category".into(),
            data_type: "String".into(),
        },
    ];
    write_wide_part(
        &fact,
        &fact_schema,
        &[
            vec![Some("1".into()), Some("east".into())],
            vec![Some("1".into()), Some("east".into())],
            vec![Some("2".into()), Some("west".into())],
            vec![Some("3".into()), Some("east".into())],
        ],
    )
    .unwrap();
    let id_schema = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    write_wide_part(
        &dimension,
        &id_schema,
        &[
            vec![Some("1".into())],
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &id_schema,
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
        ],
    )
    .unwrap();
    let paths = [fact, dimension, dates];
    let base = "SELECT f.category AS region, f.id FROM fact f JOIN dimension d ON f.id = d.id JOIN dates t ON d.id = t.id GROUP BY region, f.id ORDER BY f.id";
    let query = parse_join(base).unwrap().unwrap();
    assert!(query.grouped_keys_only);
    let mut output = Vec::new();
    execute_multi_join(&query, &paths, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "region\tid\neast\t1\nwest\t2\neast\t3\n"
    );

    let query = parse_join(&format!("{base} LIMIT 1 OFFSET 1"))
        .unwrap()
        .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &paths, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "region\tid\nwest\t2\n");

    let two_table = "SELECT f.category AS region, f.id FROM fact f, dimension d WHERE f.id = d.id GROUP BY region, f.id ORDER BY f.id";
    let query = parse_join(two_table).unwrap().unwrap();
    assert!(query.grouped_keys_only);
    let mut output = Vec::new();
    execute_multi_join(&query, &paths[..2], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "region\tid\neast\t1\nwest\t2\neast\t3\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsupported_join_group_only_forms_fail_at_parse_time() {
    for sql in [
        "SELECT a.id FROM a JOIN b ON a.id=b.id LEFT JOIN c ON b.id=c.id GROUP BY a.id",
        "SELECT a.id FROM a JOIN b ON a.id=b.id JOIN c ON b.id=c.id GROUP BY b.id",
        "SELECT a.id FROM a JOIN b ON a.id=b.id JOIN c ON b.id=c.id GROUP BY a.id ORDER BY count",
    ] {
        assert!(parse_join(sql).is_err(), "{sql}");
    }
}
