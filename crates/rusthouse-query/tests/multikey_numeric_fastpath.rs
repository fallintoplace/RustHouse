use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn multikey_sum_and_count_preserve_nullable_numeric_values() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-multikey-fast-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let columns = [
        Column { name: "region".into(), data_type: "String".into() },
        Column { name: "category".into(), data_type: "String".into() },
        Column { name: "amount".into(), data_type: "Nullable(UInt64)".into() },
    ];
    let rows = [
        vec![Some("EU".into()), Some("books".into()), Some("10".into())],
        vec![Some("EU".into()), Some("books".into()), None],
        vec![Some("US".into()), Some("games".into()), Some("3".into())],
        vec![Some("EU".into()), Some("books".into()), Some("5".into())],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let query = parse_select(
        "SELECT region, category, sum(amount), count(amount) FROM part GROUP BY region, category ORDER BY region, category"
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&root), &mut output).unwrap();
    assert_eq!(
        output,
        b"region\tcategory\tsum(amount)\tcount(amount)\nEU\tbooks\t15\t2\nUS\tgames\t3\t1\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}
