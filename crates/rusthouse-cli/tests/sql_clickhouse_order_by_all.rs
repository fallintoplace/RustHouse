use std::fs;
use std::process::Command;

#[test]
fn order_by_all_sorts_multiple_persisted_columns_and_grouped_counts() {
    let dir = std::env::temp_dir().join(format!("rusthouse-order-all-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&dir)
            .output()
            .unwrap()
    };
    assert!(sql("CREATE TABLE items (a UInt64, b UInt64)").status.success());
    assert!(sql("INSERT INTO items VALUES (1, 2), (2, 0), (1, 3), (1, 2)").status.success());
    let sorted = sql("SELECT a, b FROM items ORDER BY ALL");
    assert!(sorted.status.success(), "{sorted:?}");
    assert_eq!(sorted.stdout, b"a\tb\n1\t2\n1\t2\n1\t3\n2\t0\n");
    let grouped = sql("SELECT a, count(*) FROM items GROUP BY a ORDER BY ALL DESC");
    assert!(grouped.status.success(), "{grouped:?}");
    assert_eq!(grouped.stdout, b"a\tcount\n2\t1\n1\t3\n");
    fs::remove_dir_all(dir).unwrap();
}
