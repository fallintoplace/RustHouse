use std::process::{Command, Output};

fn query(statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--format", "json-seq"])
        .output()
        .unwrap()
}

#[test]
fn emits_rfc7464_records_with_escaped_json_values() {
    let rows = query("SELECT number FROM numbers(2)");
    assert!(rows.status.success(), "{rows:?}");
    assert_eq!(
        rows.stdout,
        b"\x1e{\"number\":\"0\"}\n\x1e{\"number\":\"1\"}\n"
    );

    let constants = query("SELECT 'a,b' AS label, NULL AS missing");
    assert!(constants.status.success(), "{constants:?}");
    assert_eq!(
        constants.stdout,
        b"\x1e{\"label\":\"a,b\",\"missing\":null}\n"
    );

    let empty = query("SELECT number FROM numbers(0)");
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"");

    let limited = query("SELECT number FROM numbers(5) LIMIT 1");
    assert!(limited.status.success(), "{limited:?}");
    assert_eq!(limited.stdout, b"\x1e{\"number\":\"0\"}\n");
}

#[test]
fn invalid_sql_does_not_produce_partial_json_sequence() {
    let invalid = query("SELECT missing FROM numbers(3)");
    assert!(!invalid.status.success(), "{invalid:?}");
}
