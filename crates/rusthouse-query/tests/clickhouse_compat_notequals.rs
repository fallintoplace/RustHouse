use rusthouse_query::{Projection, parse_select};

#[test]
fn notequals_evaluates_clickhouse_integer_comparisons() {
    for (sql, expected) in [
        ("SELECT notEquals(1, 1)", 0),
        ("SELECT notEquals(-2, -2)", 0),
        ("SELECT notEquals(7, 6)", 1),
        ("SELECT notEquals(1 + 2, 3)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn notequals_rejects_invalid_arguments() {
    for sql in [
        "SELECT notEquals()",
        "SELECT notEquals(1)",
        "SELECT notEquals(1, 2, 3)",
        "SELECT notEquals(1, 'bad')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
