use rusthouse_query::{execute, parse_select};

fn evaluate(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| error.to_string())
}

#[test]
fn mod_pow_uses_modular_integer_arithmetic() {
    for (sql, expected) in [
        ("SELECT MOD_POW(2, 10, 1000) AS result", "24"),
        ("SELECT mod_pow(-2, 3, 5) AS result", "2"),
        ("SELECT MOD_POW(5, 0, 1) AS result", "0"),
        ("SELECT MOD_POW(7, 3, 11) AS result", "2"),
        ("SELECT MOD_POW(3, 4, 7) AS result", "4"),
    ] {
        assert_eq!(
            evaluate(sql).unwrap(),
            format!("result\n{expected}\n"),
            "{sql}"
        );
    }
}

#[test]
fn mod_pow_rejects_invalid_exponent_modulus_and_arity() {
    for sql in [
        "SELECT MOD_POW(2, -1, 5)",
        "SELECT MOD_POW(2, 3, 0)",
        "SELECT MOD_POW(2, 3, -5)",
        "SELECT MOD_POW(2, 3)",
        "SELECT MOD_POW(2, 3, 4, 5)",
        "SELECT MOD_POW(2, 3, 5) FROM numbers(1)",
    ] {
        assert!(evaluate(sql).is_err(), "{sql}");
    }
}
