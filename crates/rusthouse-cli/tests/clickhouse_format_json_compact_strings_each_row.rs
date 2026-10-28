use std::process::Command;

#[test]
fn clickhouse_format_json_compact_strings_each_row_quotes_all_fields() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(2)", "--format", "JSONCompactStringsEachRow"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"[\"0\"]\n[\"1\"]\n");
}
