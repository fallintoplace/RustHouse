use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn typed_distinct_uses_dense_integer_keys_with_string_filters() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-typed-dense-distinct-{}",
        std::process::id()
    ));
    for statement in [
        "CREATE TABLE events (byte_key UInt8, signed_byte Int8, wide_key UInt16, signed_wide Int16, tag String)",
        "INSERT INTO events VALUES (255, -128, 65535, -32768, 'good'), (0, 127, 0, 32767, 'good'), (255, -128, 65535, -32768, 'good')",
        "INSERT INTO events VALUES (1, -1, 42, -1, 'good'), (9, 9, 9, 9, 'bad')",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        ("SELECT DISTINCT byte_key FROM events WHERE tag = 'good' ORDER BY byte_key", "byte_key\n0\n1\n255\n"),
        ("SELECT DISTINCT signed_byte FROM events WHERE tag = 'good' ORDER BY signed_byte", "signed_byte\n-128\n-1\n127\n"),
        ("SELECT DISTINCT wide_key FROM events WHERE tag = 'good' ORDER BY wide_key", "wide_key\n0\n42\n65535\n"),
        ("SELECT DISTINCT signed_wide FROM events WHERE tag = 'good' ORDER BY signed_wide", "signed_wide\n-32768\n-1\n32767\n"),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
