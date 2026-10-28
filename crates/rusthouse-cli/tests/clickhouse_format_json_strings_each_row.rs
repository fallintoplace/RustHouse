use std::process::Command;

#[test]
fn clickhouse_format_json_strings_each_row_quotes_all_fields() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(2)", "--format", "JSONStringsEachRow"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"{\"number\":\"0\"}\n{\"number\":\"1\"}\n");
}
