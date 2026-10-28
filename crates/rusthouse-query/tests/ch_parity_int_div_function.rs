use rusthouse_query::{Projection, parse_select};

fn number(sql: &str) -> i64 {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(value) => value,
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn computes_signed_int64_arithmetic() {
    for (sql, expected) in [
        ("SELECT intDiv(8, 3)", 2),
        ("SELECT intDiv(-8, 3)", -2),
        ("SELECT intDiv(0, 100)", 0),
    ] {
        assert_eq!(number(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_or_overflowing_arguments() {
    for sql in [
        "SELECT intDiv()",
        "SELECT intDiv(1, 0)",
        "SELECT intDiv((0 - 9223372036854775807 - 1), -1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
