use std::process::Command;

#[test]
fn unsigned_casts_preserve_width_and_convert_numeric_expressions() {
    for (sql, value) in [
        ("SELECT toUInt8('255') AS value", 255_i64),
        ("SELECT toUInt8(256) AS value", 0),
        ("SELECT toUInt8(-1) AS value", 255),
        ("SELECT toUInt16('65535') AS value", 65535),
        ("SELECT toUInt16(65536) AS value", 0),
        ("SELECT toUInt32('4294967295') AS value", 4294967295),
        ("SELECT toUInt32(-1) AS value", 4294967295),
        ("SELECT toUInt8(toUInt16('257')) AS value", 1),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", sql])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{sql}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            result.stdout,
            format!("value\n{value}\n").as_bytes(),
            "{sql}"
        );
    }
}

#[test]
fn unsigned_casts_reject_malformed_strings_and_handle_null() {
    let null = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT toUInt32(NULL) AS value"])
        .output()
        .unwrap();
    assert!(null.status.success());
    assert_eq!(null.stdout, b"value\n\\N\n");
    for sql in [
        "SELECT toUInt8('abc')",
        "SELECT toUInt16()",
        "SELECT toUInt8('1', '2')",
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", sql])
            .output()
            .unwrap();
        assert!(!result.status.success(), "{sql}");
    }
}
