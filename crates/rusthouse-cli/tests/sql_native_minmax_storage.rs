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
fn native_extrema_preserve_signed_order_and_filtered_fallback() {
    let db = std::env::temp_dir().join(format!("rusthouse-native-extrema-{}", std::process::id()));
    for query in [
        "CREATE TABLE facts (signed_value Int64, unsigned_value UInt64, flag UInt8)",
        "INSERT INTO facts VALUES (-100, 200, 0), (30, 0, 1), (200, 500, 1)",
        "INSERT INTO facts VALUES (-90, 800, 1), (7, 42, 0)",
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
    }
    for (query, expected) in [
        (
            "SELECT min(signed_value) FROM facts",
            "min(signed_value)\n-100\n",
        ),
        (
            "SELECT max(signed_value) FROM facts",
            "max(signed_value)\n200\n",
        ),
        (
            "SELECT min(unsigned_value) FROM facts",
            "min(unsigned_value)\n0\n",
        ),
        (
            "SELECT max(unsigned_value) FROM facts",
            "max(unsigned_value)\n800\n",
        ),
        (
            "SELECT min(signed_value) FROM facts WHERE flag = 1",
            "min(signed_value)\n-90\n",
        ),
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected,
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
