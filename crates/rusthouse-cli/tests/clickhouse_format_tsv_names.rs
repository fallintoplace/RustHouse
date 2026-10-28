use std::process::Command;

#[test]
fn clickhouse_format_tsv_names_matches_rows() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(2)", "--format", "TSVWithNames"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"number\n0\n1\n");
}
