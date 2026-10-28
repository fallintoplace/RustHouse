use std::process::Command;

#[test]
fn clickhouse_format_tsv_matches_rows() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(2)", "--format", "TSV"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"0\n1\n");
}
