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
        ("SELECT intDivOrZero(8, 3)", 2),
        ("SELECT intDivOrZero(-8, 3)", -2),
        ("SELECT intDivOrZero(10, 0)", 0),
        ("SELECT intDivOrZero((0 - 9223372036854775807 - 1), -1)", 0),
    ] {
        assert_eq!(number(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_wrong_arity() {
    for sql in [
        "SELECT intDivOrZero()",
        "SELECT intDivOrZero(1)",
        "SELECT intDivOrZero(1, 2, 3)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
