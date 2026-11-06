use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn filters_unmatched_rows_before_evaluating_left_join_projections() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-left-boolean-filter-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let base = root.join("base");
    let web = root.join("web");
    let catalog = root.join("catalog");

    write_wide_part(
        &base,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "year".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "qty".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("2000".into()), Some("20".into())],
            vec![Some("2".into()), Some("2000".into()), Some("10".into())],
            vec![Some("3".into()), Some("2000".into()), Some("15".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &web,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "qty".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[vec![Some("1".into()), Some("5".into())]],
    )
    .unwrap();
    write_wide_part(
        &catalog,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "qty".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[vec![Some("3".into()), Some("3".into())]],
    )
    .unwrap();

    let query = parse_join(
        "SELECT base.id, ROUND((base.qty * 1.00) / (COALESCE(web.qty, 0) + COALESCE(catalog.qty, 0)), 2) AS ratio FROM base LEFT JOIN web ON base.id = web.id LEFT JOIN catalog ON base.id = catalog.id WHERE (COALESCE(web.qty, 0) > 0 OR COALESCE(catalog.qty, 0) > 0) AND base.year = 2000 ORDER BY ratio",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[base, web, catalog], &mut output).unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "id\tratio\n1\t4\n3\t5\n"
    );
    fs::remove_dir_all(root).unwrap();
}
