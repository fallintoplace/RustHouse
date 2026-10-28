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
fn sums_native_signed_and_unsigned_integer_batches() {
    let db = std::env::temp_dir().join(format!("rusthouse-native-sum-{}", std::process::id()));
    for query in [
        "CREATE TABLE values (signed Int64, unsigned UInt64)",
        "INSERT INTO values VALUES (-100, 100), (5, 20), (200, 300)",
        "INSERT INTO values VALUES (-10, 40), (1, 0)",
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
    }
    for (query, expected) in [
        ("SELECT sum(signed) FROM values", "sum(signed)\n96\n"),
        ("SELECT sum(unsigned) FROM values", "sum(unsigned)\n460\n"),
        ("SELECT sum(signed) FROM values WHERE unsigned > 20", "sum(signed)\n90\n"),
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
