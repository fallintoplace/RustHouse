use std::process::Command;

fn sql(input: &str) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", input, "--format", "sql-values"])
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

#[test]
fn exports_sql_constants_as_values() {
    assert_eq!(
        sql("SELECT 1, 'O''Brien', NULL"),
        (true, "VALUES ('1', 'O''Brien', NULL);\n".into())
    );
    assert_eq!(
        sql("SELECT number FROM numbers(3) LIMIT 2"),
        (true, "VALUES ('0'),\n('1');\n".into())
    );
    assert_eq!(sql("SELECT number FROM numbers(0)"), (true, String::new()));
}
