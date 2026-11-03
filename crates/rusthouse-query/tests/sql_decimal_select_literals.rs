use rusthouse_query::{execute, parse_select};

fn output(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut result = Vec::new();
    execute(&query, None, &mut result).unwrap();
    String::from_utf8(result).unwrap()
}

#[test]
fn decimal_select_literals_are_numeric_constants() {
    assert_eq!(output("SELECT 1.25 AS amount"), "amount\n1.25\n");
    assert_eq!(output("SELECT -0.5 AS ratio"), "ratio\n-0.5\n");
    assert_eq!(output("SELECT 0.2, 3 AS n"), "0.2\tn\n0.2\t3\n");
}

#[test]
fn decimal_select_literals_must_be_finite() {
    for sql in ["SELECT 1e999", "SELECT -1e999"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
