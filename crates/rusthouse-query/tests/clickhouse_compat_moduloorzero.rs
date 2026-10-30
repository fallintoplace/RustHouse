use rusthouse_query::{Projection, parse_select};

#[test]
fn moduloorzero_handles_clickhouse_constant_arithmetic() {
    for (sql, expected) in [
        ("SELECT moduloOrZero(5, 2)", 1),
        ("SELECT moduloOrZero(-5, 2)", -1),
        ("SELECT moduloOrZero(5, 0)", 0),
        ("SELECT moduloOrZero(-9223372036854775807 - 1, -1)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn moduloorzero_rejects_invalid_arguments() {
    for sql in [
        "SELECT moduloOrZero()",
        "SELECT moduloOrZero(1, 2, 3)",
        "SELECT moduloOrZero('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
