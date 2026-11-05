use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn grouped_avg_rollup_emits_prefix_totals_and_orders_by_metric_alias() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-avg-rollup-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let item = root.join("item");
    let dates = root.join("dates");

    write_wide_part(
        &fact,
        &[
            Column {
                name: "item_id".into(),
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
            vec![Some("1".into()), Some("2".into()), Some("20".into())],
            vec![Some("2".into()), Some("1".into()), Some("60".into())],
            vec![Some("3".into()), Some("3".into()), Some("30".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &item,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "class".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("x".into())],
            vec![Some("2".into()), Some("y".into())],
            vec![Some("3".into()), Some("x".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &dates,
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
            vec![Some("2".into()), Some("A".into())],
            vec![Some("3".into()), Some("B".into())],
        ],
    )
    .unwrap();

    let query = parse_join(
        "SELECT d.region, i.class, AVG(f.amount) AS avg_amount FROM fact f JOIN item i ON f.item_id=i.id JOIN dates d ON f.date_id=d.id GROUP BY ROLLUP(d.region, i.class) ORDER BY avg_amount NULLS FIRST, d.region NULLS FIRST, i.class NULLS FIRST",
    )
    .unwrap()
    .unwrap();
    assert!(query.grouped_rollup);
    assert_eq!(query.order_by[0].0.name, "#avg:0");

    let mut output = Vec::new();
    execute_multi_join(&query, &[fact, item, dates], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "region\tclass\tavg_amount\nA\tx\t15\n\\N\t\\N\t30\nA\t\\N\t30\nB\t\\N\t30\nB\tx\t30\nA\ty\t60\n"
    );

    fs::remove_dir_all(root).unwrap();
}
