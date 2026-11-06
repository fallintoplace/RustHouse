use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn between_and_not_between_accept_arithmetic_bounds_from_joined_columns() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-multi-join-expression-between-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let sales = root.join("sales");
    let ranges = root.join("ranges");

    write_wide_part(
        &sales,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "value".into(),
                data_type: "Nullable(Float64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("90".into())],
            vec![Some("2".into()), Some("110".into())],
            vec![Some("3".into()), Some("111".into())],
            vec![Some("4".into()), None],
            vec![Some("5".into()), Some("100".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &ranges,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "lower_value".into(),
                data_type: "Float64".into(),
            },
            Column {
                name: "upper_value".into(),
                data_type: "Nullable(Float64)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("100".into()), Some("100".into())],
            vec![Some("2".into()), Some("100".into()), Some("100".into())],
            vec![Some("3".into()), Some("100".into()), Some("100".into())],
            vec![Some("4".into()), Some("100".into()), Some("100".into())],
            vec![Some("5".into()), Some("100".into()), None],
        ],
    )
    .unwrap();

    for (operator, expected) in [("BETWEEN", "id\n1\n2\n"), ("NOT BETWEEN", "id\n3\n")] {
        let sql = format!(
            "SELECT s.id FROM sales s JOIN ranges r ON s.id = r.id WHERE s.value {operator} 0.9 * r.lower_value AND 1.1 * r.upper_value ORDER BY s.id"
        );
        let query = parse_join(&sql).unwrap().unwrap();
        let mut output = Vec::new();
        execute_multi_join(&query, &[sales.clone(), ranges.clone()], &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{operator}");
    }

    fs::remove_dir_all(root).unwrap();
}
