use std::process::Command;

fn run(query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap()
}

#[test]
fn sorts_by_selected_column_position() {
    let result = run("SELECT number FROM numbers(6) ORDER BY 1 DESC LIMIT 3");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "number\n5\n4\n3\n"
    );
}

#[test]
fn reports_out_of_range_positions() {
    let result = run("SELECT number FROM numbers(6) ORDER BY 2");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("out of range"));
}
