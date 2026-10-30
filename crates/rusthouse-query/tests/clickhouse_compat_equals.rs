use rusthouse_query::{Projection, parse_select};

#[test]
fn equals_evaluates_clickhouse_integer_comparisons() {
    for (sql, expected) in [
        ("SELECT equals(1, 1)", 1),
        ("SELECT equals(-2, -2)", 1),
        ("SELECT equals(7, 6)", 0),
        ("SELECT equals(1 + 2, 3)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn equals_rejects_invalid_arguments() {
    for sql in [
        "SELECT equals()",
        "SELECT equals(1)",
        "SELECT equals(1, 2, 3)",
        "SELECT equals(1, 'bad')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
