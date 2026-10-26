use std::process::Command;

#[test]
fn emits_compact_json_arrays_without_column_names() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT 7 AS amount, 'hello' AS label, NULL AS missing",
            "--format",
            "json-compact-each-row",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"[\"7\",\"hello\",null]\n");
}
