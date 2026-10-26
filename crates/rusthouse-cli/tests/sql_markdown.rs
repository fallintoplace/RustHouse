use std::process::Command;

fn sql_markdown(query: &str, flags: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--format", "markdown"])
        .args(flags)
        .output()
        .unwrap()
}

#[test]
fn formats_numeric_queries_as_markdown_tables() {
    let output = sql_markdown(
        "SELECT number, number FROM numbers(3) LIMIT 2",
        &[],
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"| number | number |\n| --- | --- |\n| 0 | 0 |\n| 1 | 1 |\n"
    );
}

#[test]
fn prints_a_header_only_table_for_empty_results() {
    let output = sql_markdown("SELECT number FROM numbers(0)", &[]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"| number |\n| --- |\n");

    let capped = sql_markdown("SELECT number FROM numbers(10)", &["--max-rows", "0"]);
    assert!(capped.status.success(), "{capped:?}");
    assert_eq!(capped.stdout, b"| number |\n| --- |\n");
}

#[test]
fn markdown_can_be_written_to_a_file() {
    let path = std::env::temp_dir().join(format!("rusthouse-markdown-{}.md", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT 1", "--format", "markdown", "--output"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), b"| 1 |\n| --- |\n| 1 |\n");
    std::fs::remove_file(path).unwrap();
}
