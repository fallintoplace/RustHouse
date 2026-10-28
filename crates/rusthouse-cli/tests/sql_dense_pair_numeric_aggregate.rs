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
fn two_byte_group_keys_accumulate_multiple_nullable_metrics() {
    let db = std::env::temp_dir().join(format!("rusthouse-dense-pair-agg-{}", std::process::id()));
    for query in [
        "CREATE TABLE events (region UInt8, category Int8, amount Nullable(UInt64))",
        "INSERT INTO events VALUES (255, -128, 10), (0, 127, NULL), (255, -128, 20)",
        "INSERT INTO events VALUES (0, 127, 5), (1, -1, 7), (255, -128, NULL)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    let query = "SELECT region, category, sum(amount), min(amount) FROM events GROUP BY region, category ORDER BY region, category";
    let result = sql(query, &db);
    assert!(result.status.success(), "{query}: {result:?}");
    assert_eq!(result.stdout, b"region\tcategory\tsum(amount)\tmin(amount)\n0\t127\t5\t5\n1\t-1\t7\t7\n255\t-128\t30\t10\n");
    fs::remove_dir_all(db).unwrap();
}
