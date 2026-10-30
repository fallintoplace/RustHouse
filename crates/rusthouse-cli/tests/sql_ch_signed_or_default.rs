use std::process::Command;

#[test]
fn signed_or_default_supports_clickhouse_conversions() {
    for (query, expected) in [
        ("SELECT toInt8OrDefault('128') AS value", "value\n-128\n"),
        ("SELECT toInt8OrDefault('bad', 42) AS value", "value\n42\n"),
        ("SELECT toInt8OrDefault('bad') AS value", "value\n0\n"),
        ("SELECT toInt16OrDefault('65535') AS value", "value\n-1\n"),
        ("SELECT toInt16OrDefault('invalid', -1) AS value", "value\n-1\n"),
        ("SELECT toInt32OrDefault('4294967295') AS value", "value\n-1\n"),
        ("SELECT toInt64OrDefault('42') AS value", "value\n42\n"),
        ("SELECT toInt64OrDefault('oops', -9) AS value", "value\n-9\n"),
        ("SELECT toInt8OrDefault(NULL,42) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn signed_or_default_rejects_bad_arguments() {
    for query in [
        "SELECT toInt8OrDefault()",
        "SELECT toInt8OrDefault('bad',256)",
        "SELECT toInt16OrDefault('bad',65536)",
        "SELECT toInt8OrDefault('1',1,2)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
