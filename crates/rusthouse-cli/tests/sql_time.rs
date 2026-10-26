use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn shows_elapsed_time_on_stderr_without_modifying_query_output() {
    let plain = run(&["sql", "SELECT number FROM numbers(3)"]);
    let timed = run(&["sql", "SELECT number FROM numbers(3)", "--time"]);
    assert!(plain.status.success(), "{plain:?}");
    assert!(timed.status.success(), "{timed:?}");
    assert_eq!(plain.stdout, timed.stdout);
    assert!(plain.stderr.is_empty());

    let stderr = String::from_utf8(timed.stderr).unwrap();
    let ms = stderr
        .trim()
        .strip_prefix("Elapsed: ")
        .unwrap()
        .strip_suffix(" ms")
        .unwrap()
        .parse::<f64>()
        .unwrap();
    assert!(ms.is_finite() && ms >= 0.0);
}

#[test]
fn measures_csv_output_without_polluting_the_export() {
    let path = std::env::temp_dir().join(format!("rusthouse-timing-{}.csv", std::process::id()));
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT number FROM numbers(2)", "--format", "csv", "--time", "--output"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).starts_with("Elapsed: "));
    assert_eq!(std::fs::read(&path).unwrap(), b"number\r\n0\r\n1\r\n");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn rejects_duplicate_timing_and_ast_combination() {
    let duplicate = run(&["sql", "SELECT 1", "--time", "--time"]);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate --time"));

    let ast = run(&["sql", "SELECT 1", "--time", "--show-ast"]);
    assert!(!ast.status.success());
    assert!(String::from_utf8_lossy(&ast.stderr).contains("cannot be combined"));
}

#[test]
fn failed_queries_keep_error_reporting_and_emit_timing() {
    let result = run(&["sql", "SELECT missing FROM numbers(1)", "--time"]);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("Elapsed: "));
    assert!(stderr.contains("error: "));
}
