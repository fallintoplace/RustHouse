use rusthouse_query::{Projection, parse_select};

#[test]
fn iszero_handles_clickhouse_constant_arithmetic() {
    for (sql, expected) in [
        ("SELECT isZero(0)", 1),
        ("SELECT isZero(1)", 0),
        ("SELECT isZero(-1)", 0),
        ("SELECT isZero(4 - 4)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn iszero_rejects_invalid_arguments() {
    for sql in [
        "SELECT isZero()",
        "SELECT isZero(1, 2, 3)",
        "SELECT isZero('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
