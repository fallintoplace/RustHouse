use rusthouse_query::{execute_bounded_except, parse_bounded_except};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn bounded_except_deduplicates_left_rows_and_subtracts_multiple_channels() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-except-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let names = ["store", "catalog", "web"];
    let columns = ["store_id", "catalog_id", "web_id"];
    let input = [vec![1, 2, 2, 3, 4], vec![2], vec![4]];
    let paths = names
        .iter()
        .zip(columns)
        .zip(input)
        .map(|((table, column), values)| {
            let path = root.join(table);
            let rows = values
                .into_iter()
                .map(|value| vec![Some(value.to_string())])
                .collect::<Vec<_>>();
            write_wide_part(
                &path,
                &[Column {
                    name: column.into(),
                    data_type: "UInt64".into(),
                }],
                &rows,
            )
            .unwrap();
            path
        })
        .collect::<Vec<_>>();
    let sql = "SELECT store_id FROM store EXCEPT SELECT catalog_id FROM catalog EXCEPT SELECT web_id FROM web";
    let plan = parse_bounded_except(sql).unwrap().unwrap();
    let mut out = Vec::new();
    execute_bounded_except(&plan, &paths, &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "store_id\n1\n3\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsupported_except_shapes_fail_closed() {
    for sql in [
        "SELECT x FROM a EXCEPT ALL SELECT x FROM b",
        "SELECT x FROM a EXCEPT SELECT x FROM b EXCEPT SELECT x FROM c EXCEPT SELECT x FROM d",
        "SELECT x FROM a EXCEPT SELECT count(*) FROM b",
        "SELECT x FROM a ORDER BY x EXCEPT SELECT x FROM b",
        "SELECT x FROM a EXCEPT SELECT x FROM b LIMIT 1",
        "SELECT number FROM numbers(3) EXCEPT SELECT number FROM numbers(4)",
    ] {
        assert!(parse_bounded_except(sql).is_err(), "{sql}");
    }
    assert!(parse_bounded_except("SELECT x FROM a").unwrap().is_none());
}
