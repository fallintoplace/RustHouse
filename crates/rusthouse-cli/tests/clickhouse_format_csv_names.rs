use std::process::Command;

#[test]
fn clickhouse_format_csv_names_matches_rows() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(2)", "--format", "CSVWithNames"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"number\r\n0\r\n1\r\n");
}
