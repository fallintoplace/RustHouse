use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn scalar_abs_and_sign_support_aliases_and_composition() {
    assert_eq!(evaluate("SELECT ABS(-7) AS magnitude").unwrap(), "magnitude\n7\n");
    assert_eq!(evaluate("SELECT sign(-10), SIGN(0), sign(9)").unwrap(), "sign(-10)\tsign(0)\tsign(9)\n-1\t0\t1\n");
    assert_eq!(evaluate("SELECT abs(-4) + sign(-9)").unwrap(), "3\n3\n");
    assert_eq!(evaluate("SELECT abs(-(2 + 3))").unwrap(), "5\n5\n");
}

#[test]
fn scalar_functions_fail_closed_on_invalid_inputs() {
    for sql in [
        "SELECT abs(-9223372036854775808)",
        "SELECT ABS()",
        "SELECT ABS(1, 2)",
        "SELECT ABS(1",
        "SELECT SIGN(1) FROM numbers(2)",
        "SELECT ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(ABS(1)))))))))))))))))",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
