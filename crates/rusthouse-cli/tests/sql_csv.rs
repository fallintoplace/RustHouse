use std::process::Command;

#[test]
fn prints_csv_for_numeric_queries() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT number, number FROM numbers(3) LIMIT 2",
            "--format",
            "csv",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "number,number\r\n0,0\r\n1,1\r\n"
    );
}

#[test]
fn refuses_unknown_output_formats() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT 1", "--format", "xml"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("tsv"));
    assert!(error.contains("csv"));
    assert!(error.contains("jsonl"));
    assert!(error.contains("json"));
}
