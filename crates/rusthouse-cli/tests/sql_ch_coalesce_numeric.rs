use std::process::Command;

#[test]
fn coalesce_supports_integer_and_string_constants() {
    for (sql, expected) in [
        ("SELECT coalesce(NULL, 42) AS value", "value\n42\n"),
        ("SELECT coalesce(0, 9) AS value", "value\n0\n"),
        ("SELECT coalesce(NULL, NULL, -7) AS value", "value\n-7\n"),
        ("SELECT coalesce(NULL, 'text') AS value", "value\ntext\n"),
        ("SELECT coalesce(NULL, NULL) AS value", "value\n\\N\n"),
        ("SELECT coalesce(NULL, toInt64OrZero('23')) AS value", "value\n23\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse")).args(["sql", sql]).output().unwrap();
        assert!(output.status.success(), "{sql}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{sql}");
    }
}

#[test]
fn coalesce_rejects_unrelated_constant_types() {
    for sql in ["SELECT coalesce(1, 'x')", "SELECT coalesce('x', 1)"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse")).args(["sql", sql]).output().unwrap();
        assert!(!output.status.success(), "{sql}");
    }
}
