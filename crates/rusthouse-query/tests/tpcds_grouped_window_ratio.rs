use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_grouped_sum_can_divide_by_its_partition_total() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-window-ratio-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("sales");
    let item = root.join("item");
    let dates = root.join("dates");

    write_wide_part(
        &sales,
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
                data_type: "Nullable(Float64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into()), Some("10".into())],
            vec![Some("1".into()), Some("2".into()), Some("15".into())],
            vec![Some("1".into()), Some("2".into()), None],
            vec![Some("2".into()), Some("1".into()), Some("75".into())],
            vec![Some("3".into()), Some("2".into()), Some("10".into())],
            vec![Some("3".into()), Some("1".into()), Some("30".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &item,
        &[
            Column {
                name: "item_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "item_class".into(),
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
    write_wide_part(
        &dates,
        &[Column {
            name: "date_id".into(),
            data_type: "UInt64".into(),
        }],
        &[vec![Some("1".into())], vec![Some("2".into())]],
    )
    .unwrap();

    let query = parse_join(
        "SELECT i.item_id, i.item_class, SUM(s.amount) AS total_amount, SUM(s.amount)*100.0000/SUM(SUM(s.amount)) OVER (PARTITION BY i.item_class) AS share FROM sales s JOIN item i ON s.item_id=i.item_id JOIN dates d ON s.date_id=d.date_id GROUP BY i.item_id, i.item_class ORDER BY i.item_class, share DESC, i.item_id",
    )
    .unwrap()
    .unwrap();
    assert!(query.grouped_window_ratio.is_some());

    let mut output = Vec::new();
    execute_multi_join(&query, &[sales, item, dates], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "item_id\titem_class\ttotal_amount\tshare\n2\tA\t75\t75\n1\tA\t25\t25\n3\tB\t40\t100\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn grouped_window_ratio_requires_a_grouped_partition_key() {
    let sql = "SELECT i.item_id, SUM(s.amount) AS total_amount, SUM(s.amount)*100.0/SUM(SUM(s.amount)) OVER (PARTITION BY d.date_id) AS share FROM sales s JOIN item i ON s.item_id=i.item_id JOIN dates d ON s.date_id=d.date_id GROUP BY i.item_id";
    assert!(parse_join(sql).is_err());
}
