use std::fs;
use std::process::Command;

#[test]
fn nullable_sixteen_bit_distinct_uses_exact_bitmap_with_filters() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-nullable-u16-distinct-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (signed_key Nullable(Int16), positive_key Nullable(UInt16), flag UInt8)",
        "INSERT INTO events VALUES (-32768, 65535, 1), (-32768, 65535, 1), (32767, 0, 0), (NULL, NULL, 1)",
        "INSERT INTO events VALUES (-1, 42, 1), (32767, 0, 1), (NULL, 65535, 1)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (statement, expected) in [
        ("SELECT count(DISTINCT signed_key) FROM events", b"countDistinct(signed_key)\n3\n".as_slice()),
        ("SELECT count(DISTINCT signed_key) FROM events WHERE flag = 1", b"countDistinct(signed_key)\n3\n".as_slice()),
        ("SELECT count(DISTINCT positive_key) FROM events", b"countDistinct(positive_key)\n3\n".as_slice()),
        ("SELECT count(DISTINCT positive_key) FROM events WHERE flag = 0", b"countDistinct(positive_key)\n1\n".as_slice()),
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(result.stdout.as_slice(), expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
