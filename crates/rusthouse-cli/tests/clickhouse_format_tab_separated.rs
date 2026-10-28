use std::process::Command;

#[test]
fn clickhouse_tab_separated_outputs_expected_rows() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(2)", "--format", "TabSeparated"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"0\n1\n");
}
