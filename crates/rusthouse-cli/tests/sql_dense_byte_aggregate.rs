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
fn dense_byte_groups_preserve_numeric_aggregate_results() {
    let db = std::env::temp_dir().join(format!("rusthouse-dense-byte-agg-{}", std::process::id()));
    for statement in [
        "CREATE TABLE measurements (signed_key Int8, unsigned_key UInt8, amount Nullable(UInt64))",
        "INSERT INTO measurements VALUES (-128, 255, 10), (-128, 255, 20), (127, 0, NULL)",
        "INSERT INTO measurements VALUES (127, 0, 3), (-128, 255, NULL), (0, 1, 5)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT signed_key, sum(amount) FROM measurements GROUP BY signed_key ORDER BY signed_key",
            "signed_key\tsum(amount)\n-128\t30\n0\t5\n127\t3\n",
        ),
        (
            "SELECT unsigned_key, max(amount) FROM measurements GROUP BY unsigned_key ORDER BY unsigned_key",
            "unsigned_key\tmax(amount)\n0\t3\n1\t5\n255\t20\n",
        ),
        (
            "SELECT signed_key, sum(amount) FROM measurements WHERE amount IS NOT NULL GROUP BY signed_key ORDER BY signed_key",
            "signed_key\tsum(amount)\n-128\t30\n0\t5\n127\t3\n",
        ),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            expected,
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
