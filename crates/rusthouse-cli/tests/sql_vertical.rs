use std::process::Command;

fn sql_vertical(query: &str, options: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--format", "vertical"])
        .args(options)
        .output()
        .unwrap()
}

#[test]
fn prints_numeric_sql_results_in_vertical_format() {
    let result = sql_vertical("SELECT number FROM numbers(3) LIMIT 2", &[]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"Row 1:\n  number: 0\n\nRow 2:\n  number: 1\n\n"
    );
}

#[test]
fn vertical_format_handles_multiple_projected_columns() {
    let result = sql_vertical("SELECT number, number FROM numbers(2) LIMIT 1", &[]);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"Row 1:\n  number: 0\n  number: 0\n\n"
    );
}

#[test]
fn vertical_format_prints_nothing_for_no_rows() {
    let result = sql_vertical("SELECT number FROM numbers(0)", &[]);
    assert!(result.status.success(), "{result:?}");
    assert!(result.stdout.is_empty());
}

#[test]
fn vertical_format_respects_output_file() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-sql-vertical-{}.txt",
        std::process::id()
    ));
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT 1", "--format", "vertical", "--output"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stdout.is_empty());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"Row 1:\n  1: 1\n\n"
    );
    std::fs::remove_file(path).unwrap();
}
