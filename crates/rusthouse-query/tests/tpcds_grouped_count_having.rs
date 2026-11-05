use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn grouped_join_count_having_filters_groups_before_order_and_limit() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-count-having-{}-{}",
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
                name: "dimension_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "date_id".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into())],
            vec![Some("1".into()), Some("2".into())],
            vec![Some("2".into()), Some("1".into())],
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
                name: "region".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("A".into())],
            vec![Some("2".into()), Some("B".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &dates,
        &[Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }],
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();

    let query = parse_join(
        "SELECT d.region, COUNT(*) AS cnt FROM fact f JOIN dimension d ON f.dimension_id=d.id JOIN dates t ON f.date_id=t.id GROUP BY d.region HAVING COUNT(*) >= 2 ORDER BY cnt DESC, d.region LIMIT 1",
    )
    .unwrap()
    .unwrap();
    assert!(query.grouped_count_having.is_some());

    let mut output = Vec::new();
    execute_multi_join(&query, &[fact, dimension, dates], &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "region\tcnt\nA\t2\n");

    fs::remove_dir_all(root).unwrap();
}
