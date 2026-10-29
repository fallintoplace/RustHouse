use rusthouse_query::{Projection, parse_select};

#[test]
fn intdivorzero_handles_clickhouse_constant_arithmetic() {
    for (sql, expected) in [
        ("SELECT intDivOrZero(5, 2)", 2),
        ("SELECT intDivOrZero(-5, 2)", -2),
        ("SELECT intDivOrZero(5, 0)", 0),
        ("SELECT intDivOrZero(-9223372036854775807 - 1, -1)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn intdivorzero_rejects_invalid_arguments() {
    for sql in [
        "SELECT intDivOrZero()",
        "SELECT intDivOrZero(1, 2, 3)",
        "SELECT intDivOrZero('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
