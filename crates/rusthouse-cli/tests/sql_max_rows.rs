use std::process::Command;

fn sql(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("sql")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn caps_generated_rows_without_changing_sql_limit_or_offset() {
    let result = sql(&["SELECT number FROM numbers(10)", "--max-rows", "2"]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"number\n0\n1\n");

    let result = sql(&[
        "SELECT number FROM numbers(10) LIMIT 4 OFFSET 3",
        "--max-rows",
        "2",
    ]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"number\n3\n4\n");

    let result = sql(&["SELECT number FROM numbers(10) LIMIT 1", "--max-rows", "3"]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"number\n0\n");
}

#[test]
fn zero_cap_preserves_header_and_caps_aggregate_results() {
    let result = sql(&["SELECT number FROM numbers(10)", "--max-rows", "0"]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"number\n");

    let result = sql(&["SELECT count() FROM numbers(10)", "--max-rows", "0"]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"count\n");
}

#[test]
fn rejects_invalid_duplicate_and_missing_caps() {
    for args in [
        vec!["SELECT 1", "--max-rows"],
        vec!["SELECT 1", "--max-rows", "-1"],
        vec!["SELECT 1", "--max-rows", "1.5"],
        vec!["SELECT 1", "--max-rows", "1", "--max-rows", "2"],
        vec!["SELECT 1", "--max-rows", "9999999999999999999999999999"],
    ] {
        let result = sql(&args);
        assert!(!result.status.success(), "{args:?}");
    }
}
