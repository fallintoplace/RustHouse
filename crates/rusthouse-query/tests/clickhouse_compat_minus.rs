use rusthouse_query::{Projection, parse_select};

#[test]
fn minus_folds_clickhouse_numeric_constants() {
    for (sql, expected) in [
        ("SELECT minus(1, 2)", -1),
        ("SELECT minus(-5, -2)", -3),
        ("SELECT minus(3 * 4, 1 + 2)", 9),
        ("SELECT minus(7, 7)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn minus_rejects_invalid_arguments() {
    for sql in [
        "SELECT minus()",
        "SELECT minus(1, 2, 3)",
        "SELECT minus('invalid')",
        "SELECT minus(-9223372036854775807, 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
