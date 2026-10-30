use rusthouse_query::{Projection, parse_select};

#[test]
fn plus_folds_clickhouse_numeric_constants() {
    for (sql, expected) in [
        ("SELECT plus(1, 2)", 3),
        ("SELECT plus(-5, 2)", -3),
        ("SELECT plus(1 + 2, 3 * 4)", 15),
        ("SELECT plus(-7, 7)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn plus_rejects_invalid_arguments() {
    for sql in [
        "SELECT plus()",
        "SELECT plus(1, 2, 3)",
        "SELECT plus('invalid')",
        "SELECT plus(9223372036854775807, 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
