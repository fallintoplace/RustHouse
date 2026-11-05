use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_grouped_join_emits_multiple_independent_sums() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-tpcds-multi-sum-{}-{}",
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
                name: "sales_profit".into(),
                data_type: "Nullable(Int64)".into(),
            },
            Column {
                name: "return_loss".into(),
                data_type: "Nullable(Int64)".into(),
            },
        ],
        &[
            vec![
                Some("1".into()),
                Some("1".into()),
                Some("8".into()),
                Some("2".into()),
            ],
            vec![
                Some("1".into()),
                Some("2".into()),
                Some("5".into()),
                Some("3".into()),
            ],
            vec![
                Some("2".into()),
                Some("1".into()),
                Some("10".into()),
                Some("1".into()),
            ],
        ],
    )
    .unwrap();
    write_wide_part(
        &item,
        &[Column {
            name: "item_id".into(),
            data_type: "UInt64".into(),
        }],
        &[vec![Some("1".into())], vec![Some("2".into())]],
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
        "SELECT i.item_id, SUM(s.sales_profit) AS sales_total, SUM(s.return_loss) AS return_total FROM sales s JOIN item i ON s.item_id=i.item_id JOIN dates d ON s.date_id=d.date_id GROUP BY i.item_id ORDER BY i.item_id",
    )
    .unwrap()
    .unwrap();
    assert_eq!(query.grouped_sum_extras.len(), 1);

    let mut output = Vec::new();
    execute_multi_join(&query, &[sales, item, dates], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "item_id\tsales_total\treturn_total\n1\t13\t5\n2\t10\t1\n"
    );
    fs::remove_dir_all(root).unwrap();
}
