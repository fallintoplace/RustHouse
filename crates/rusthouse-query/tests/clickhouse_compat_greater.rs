use rusthouse_query::{Projection, parse_select};

#[test]
fn greater_evaluates_clickhouse_integer_comparisons() {
    for (sql, expected) in [
        ("SELECT greater(2, 1)", 1),
        ("SELECT greater(0, -2)", 1),
        ("SELECT greater(6, 7)", 0),
        ("SELECT greater(3, 3)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn greater_rejects_invalid_arguments() {
    for sql in [
        "SELECT greater()",
        "SELECT greater(1)",
        "SELECT greater(1, 2, 3)",
        "SELECT greater(1, 'bad')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
