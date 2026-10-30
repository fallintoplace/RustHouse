use rusthouse_query::{Projection, parse_select};

#[test]
fn less_evaluates_clickhouse_integer_comparisons() {
    for (sql, expected) in [
        ("SELECT less(1, 2)", 1),
        ("SELECT less(-2, 0)", 1),
        ("SELECT less(7, 6)", 0),
        ("SELECT less(1 + 2, 3)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn less_rejects_invalid_arguments() {
    for sql in [
        "SELECT less()",
        "SELECT less(1)",
        "SELECT less(1, 2, 3)",
        "SELECT less(1, 'bad')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
