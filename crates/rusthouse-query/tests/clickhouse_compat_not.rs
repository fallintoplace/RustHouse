use rusthouse_query::{Projection, parse_select};

#[test]
fn not_folds_clickhouse_numeric_constants() {
    for (sql, expected) in [
        ("SELECT not(0)", 1),
        ("SELECT not(1)", 0),
        ("SELECT not(-1)", 0),
        ("SELECT not(4 - 4)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn not_rejects_invalid_arguments() {
    for sql in [
        "SELECT not()",
        "SELECT not(1, 2, 3)",
        "SELECT not('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
