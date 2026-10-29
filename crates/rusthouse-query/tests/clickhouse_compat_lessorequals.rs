use rusthouse_query::{Projection, parse_select};

#[test]
fn lessorequals_evaluates_clickhouse_integer_comparisons() {
    for (sql, expected) in [
        ("SELECT lessOrEquals(1, 2)", 1),
        ("SELECT lessOrEquals(-2, -2)", 1),
        ("SELECT lessOrEquals(7, 6)", 0),
        ("SELECT lessOrEquals(3, 3)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn lessorequals_rejects_invalid_arguments() {
    for sql in [
        "SELECT lessOrEquals()",
        "SELECT lessOrEquals(1)",
        "SELECT lessOrEquals(1, 2, 3)",
        "SELECT lessOrEquals(1, 'bad')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
