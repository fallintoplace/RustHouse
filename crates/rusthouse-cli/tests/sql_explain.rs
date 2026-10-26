use std::process::Command;

#[test]
fn explains_a_query_without_opening_the_part_directory() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args([
            "sql",
            "SELECT number FROM numbers(10) WHERE number >= 5 ORDER BY number DESC LIMIT 3",
            "--explain",
            "--part",
            "/this/path/does/not/exist",
            "--max-rows",
            "2",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("Logical plan (no data read):\n"));
    assert!(text.contains("Filter: 1 conditions"));
    assert!(text.contains("Sort: number DESC"));
    assert!(text.contains("Limit: 2"));
}

#[test]
fn rejects_conflicting_explain_options() {
    for option in ["--show-ast", "--time", "--format", "--output"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rusthouse"));
        command.args(["sql", "SELECT 1", "--explain", option]);
        if option == "--format" {
            command.arg("csv");
        }
        if option == "--output" {
            command.arg("out.txt");
        }
        let result = command.output().unwrap();
        assert!(!result.status.success(), "{option}");
    }
}
