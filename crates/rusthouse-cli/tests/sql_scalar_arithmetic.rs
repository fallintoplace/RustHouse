use std::process::Command;

fn sql(statement: &str) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement])
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

#[test]
fn constant_select_evaluates_precedence_and_aliases() {
    assert_eq!(
        sql("SELECT (2 + 3) * 4 AS result"),
        (true, "result\n20\n".into())
    );
    assert_eq!(sql("SELECT 3 - 7"), (true, "-4\n-4\n".into()));
    assert_eq!(sql("SELECT 10 % 3"), (true, "1\n1\n".into()));
    assert_eq!(
        sql("SELECT -9223372036854775808"),
        (true, "-9223372036854775808\n-9223372036854775808\n".into())
    );
}

#[test]
fn arithmetic_overflow_and_zero_modulo_are_errors() {
    assert!(!sql("SELECT 9223372036854775807 + 1").0);
    assert!(!sql("SELECT 4 % 0").0);
    assert!(!sql("SELECT 1 +").0);
}
