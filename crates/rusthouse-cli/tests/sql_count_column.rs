use std::process::Command;

fn query(sql: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("sql")
        .arg(sql)
        .output()
        .unwrap()
}

#[test]
fn counts_generated_column_values_with_filters_and_pagination() {
    for (sql, expected) in [
        ("SELECT count(number) FROM numbers(5)", "count(number)\n5\n"),
        (
            "SELECT count(number) AS total FROM numbers(10) WHERE number BETWEEN 3 AND 6",
            "total\n4\n",
        ),
        ("SELECT count(number) FROM numbers(0)", "count(number)\n0\n"),
        (
            "SELECT count(number) FROM numbers(10) LIMIT 0",
            "count(number)\n",
        ),
        (
            "SELECT count(number) FROM numbers(10) LIMIT 1 OFFSET 1",
            "count(number)\n",
        ),
    ] {
        let output = query(sql);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
}

#[test]
fn rejects_missing_generated_columns() {
    let output = query("SELECT count(missing) FROM numbers(5)");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("only the number column"));
}
