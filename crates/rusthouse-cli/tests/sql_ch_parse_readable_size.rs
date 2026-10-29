use std::process::Command;

#[test]
fn parse_readable_size_supports_clickhouse_conversions() {
    for (query, expected) in [
        ("SELECT parseReadableSize('1 B') AS value", "value\n1\n"),
        ("SELECT parseReadableSize('1 KiB') AS value", "value\n1024\n"),
        ("SELECT parseReadableSize('3 MB') AS value", "value\n3000000\n"),
        ("SELECT parseReadableSize('5.314 KiB') AS value", "value\n5442\n"),
        ("SELECT parseReadableSize('1.001 B') AS value", "value\n2\n"),
        ("SELECT parseReadableSize('9223372036854775807 B') AS value", "value\n9223372036854775807\n"),
        ("SELECT parseReadableSizeOrZero('invalid') AS value", "value\n0\n"),
        ("SELECT parseReadableSizeOrNull('invalid') AS value", "value\n\\N\n"),
        ("SELECT parseReadableSizeOrZero(NULL) AS value", "value\n\\N\n"),
        ("SELECT parseReadableSizeOrNull('2 KiB') AS value", "value\n2048\n"),
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
fn parse_readable_size_rejects_bad_arguments() {
    for query in [
        "SELECT parseReadableSize('bad')",
        "SELECT parseReadableSize('1 XYZ')",
        "SELECT parseReadableSize('-1 B')",
        "SELECT parseReadableSize('99999999999999999999999 B')",
        "SELECT parseReadableSize()",
        "SELECT parseReadableSize('1 KiB',2)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
