use std::process::Command;

#[test]
fn unsigned_or_default_supports_clickhouse_conversions() {
    for (query, expected) in [
        ("SELECT toUInt8OrDefault('255') AS value", "value\n255\n"),
        ("SELECT toUInt8OrDefault('256') AS value", "value\n0\n"),
        ("SELECT toUInt8OrDefault('bad', 7) AS value", "value\n7\n"),
        ("SELECT toUInt8OrDefault('bad') AS value", "value\n0\n"),
        ("SELECT toUInt16OrDefault('65535') AS value", "value\n65535\n"),
        ("SELECT toUInt16OrDefault('-1', 9) AS value", "value\n9\n"),
        ("SELECT toUInt32OrDefault('4294967295') AS value", "value\n4294967295\n"),
        ("SELECT toUInt32OrDefault('bad', 123) AS value", "value\n123\n"),
        ("SELECT toUInt8OrDefault(NULL,7) AS value", "value\n\\N\n"),
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
fn unsigned_or_default_rejects_bad_arguments() {
    for query in [
        "SELECT toUInt8OrDefault()",
        "SELECT toUInt8OrDefault('bad',-1)",
        "SELECT toUInt8OrDefault('bad',256)",
        "SELECT toUInt16OrDefault('bad',65536)",
        "SELECT toUInt8OrDefault('1',1,2)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
