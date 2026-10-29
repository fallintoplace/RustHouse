use rusthouse_query::{Projection, parse_select};

#[test]
fn positive_preserves_signed_integer_constants() {
    for (sql, expected) in [
        ("SELECT positive(0)", 0),
        ("SELECT positive(-7)", -7),
        ("SELECT positive(42)", 42),
        ("SELECT positive(7 + 2)", 9),
        ("SELECT positive(-9223372036854775807 - 1)", i64::MIN),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn positive_rejects_invalid_arguments() {
    for sql in [
        "SELECT positive()",
        "SELECT positive(1, 2)",
        "SELECT positive('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
