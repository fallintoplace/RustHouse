use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(query: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn dense_byte_multi_aggregate_groups_handle_nullable_values() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-dense-byte-multi-{}", std::process::id()));
    for query in [
        "CREATE TABLE metrics (category Int8, amount Nullable(UInt64))",
        "INSERT INTO metrics VALUES (-128, 10), (127, 1), (-128, NULL)",
        "INSERT INTO metrics VALUES (-128, 20), (127, 3), (0, 7)",
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
    }
    let query = "SELECT category, sum(amount), min(amount), max(amount) FROM metrics GROUP BY category ORDER BY category";
    let result = sql(query, &db);
    assert!(result.status.success(), "{query}: {result:?}");
    assert_eq!(result.stdout, b"category\tsum(amount)\tmin(amount)\tmax(amount)\n-128\t30\t10\t20\n0\t7\t7\t7\n127\t4\t1\t3\n");
    fs::remove_dir_all(db).unwrap();
}
