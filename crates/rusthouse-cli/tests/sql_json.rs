use std::process::Command;

#[test]
fn json_format_exports_numbers_as_strings() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(3)", "--format", "json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "[{\"number\":\"0\"},{\"number\":\"1\"},{\"number\":\"2\"}]\n"
    );
}

#[test]
fn json_format_handles_empty_results_without_rows() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(0)", "--format", "json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"[]\n");
}

#[test]
fn json_format_matches_large_integer_precision_and_filters() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(18446744073709551612, 3) WHERE number > 18446744073709551612", "--format", "json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "[{\"number\":\"18446744073709551613\"},{\"number\":\"18446744073709551614\"}]\n"
    );
}
