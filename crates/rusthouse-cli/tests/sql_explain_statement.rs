use std::process::Command;

fn sql(statement: &str, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement])
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn explain_sql_statement_plans_without_opening_data() {
    let result = sql(
        "EXPLAIN SELECT number FROM numbers(10) WHERE number >= 5 ORDER BY number DESC LIMIT 3",
        &["--part", "/path/that/does/not/exist", "--max-rows", "2"],
    );
    assert!(result.status.success(), "{result:?}");
    let plan = String::from_utf8(result.stdout).unwrap();
    assert!(plan.starts_with("Logical plan (no data read):\n"));
    assert!(plan.contains("Filter: 1 conditions"));
    assert!(plan.contains("Sort: number DESC"));
    assert!(plan.contains("Limit: 2"));

    let lower = sql("explain select 42", &[]);
    assert!(lower.status.success(), "{lower:?}");
    assert!(
        String::from_utf8(lower.stdout)
            .unwrap()
            .contains("Project: 42")
    );
}

#[test]
fn explain_statement_rejects_nonselect_and_conflicting_options() {
    for (statement, options) in [
        ("EXPLAIN", &[][..]),
        ("EXPLAIN DELETE FROM t", &[][..]),
        ("EXPLAIN SELECT 1", &["--explain"][..]),
        ("EXPLAIN SELECT 1", &["--time"][..]),
    ] {
        let result = sql(statement, options);
        assert!(!result.status.success(), "{statement}: {result:?}");
    }
}
