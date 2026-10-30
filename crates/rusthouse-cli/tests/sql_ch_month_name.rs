use std::process::Command;

#[test]
fn month_name_supports_boundaries() {
    for (query, expected) in [
        ("SELECT monthName('2024-01-01') AS value", "value\nJanuary\n"),
        ("SELECT monthName('2021-04-14 11:22:33') AS value", "value\nApril\n"),
        ("SELECT monthName('2024-12-31') AS value", "value\nDecember\n"),
        ("SELECT monthName('1900-02-01') AS value", "value\nFebruary\n"),
        ("SELECT monthName(NULL) AS value", "value\n\\N\n"),
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
fn month_name_rejects_unsupported_inputs() {
    for query in [
        "SELECT monthName()",
        "SELECT monthName('2023-02-29')",
        "SELECT monthName('2024-01-01',1)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
