use rusthouse_query::{Projection, parse_select};

#[test]
fn greaterorequals_evaluates_clickhouse_integer_comparisons() {
    for (sql, expected) in [
        ("SELECT greaterOrEquals(2, 1)", 1),
        ("SELECT greaterOrEquals(-2, -2)", 1),
        ("SELECT greaterOrEquals(6, 7)", 0),
        ("SELECT greaterOrEquals(3, 3)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn greaterorequals_rejects_invalid_arguments() {
    for sql in [
        "SELECT greaterOrEquals()",
        "SELECT greaterOrEquals(1)",
        "SELECT greaterOrEquals(1, 2, 3)",
        "SELECT greaterOrEquals(1, 'bad')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
