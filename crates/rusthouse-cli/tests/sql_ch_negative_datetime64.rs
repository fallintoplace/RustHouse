use std::process::Command;

#[test]
fn datetime64_conversions_support_negative_unix_ticks() {
    for (query, expected) in [
        (
            "SELECT fromUnixTimestamp64Milli(-1) AS value",
            "1969-12-31 23:59:59.999",
        ),
        (
            "SELECT fromUnixTimestamp64Milli(-1001) AS value",
            "1969-12-31 23:59:58.999",
        ),
        (
            "SELECT fromUnixTimestamp64Micro(-1001) AS value",
            "1969-12-31 23:59:59.998999",
        ),
        (
            "SELECT fromUnixTimestamp64Nano(-1) AS value",
            "1969-12-31 23:59:59.999999999",
        ),
        (
            "SELECT fromUnixTimestamp64Milli(1001) AS value",
            "1970-01-01 00:00:01.001",
        ),
        (
            "SELECT fromUnixTimestamp64Nano(0) AS value",
            "1970-01-01 00:00:00.000000000",
        ),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            result.stdout,
            format!("value\n{expected}\n").as_bytes(),
            "{query}"
        );
    }
}

#[test]
fn datetime64_renderer_rejects_out_of_calendar_bounds() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT fromUnixTimestamp64Milli(-9223372036854775808)",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
}
