use rusthouse_query::{Projection, parse_select};

fn number(sql: &str) -> i64 {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(value) => value,
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn handles_signed_division_and_invalid_divisors() {
    for (sql, expected) in [
        ("SELECT moduloOrZero(8, 3)", 2),
        ("SELECT moduloOrZero(-8, 3)", -2),
        ("SELECT moduloOrZero(10, 0)", 0),
        ("SELECT moduloOrZero((0 - 9223372036854775807 - 1), -1)", 0),
    ] {
        assert_eq!(number(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_wrong_arity() {
    for sql in [
        "SELECT moduloOrZero()",
        "SELECT moduloOrZero(1)",
        "SELECT moduloOrZero(1, 2, 3)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
