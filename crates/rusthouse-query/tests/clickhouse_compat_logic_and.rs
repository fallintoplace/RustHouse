use rusthouse_query::{Projection, parse_select};

#[test]
fn and_folds_boolean_constant_arguments() {
    for (sql, expected) in [
        ("SELECT and(1, 2)", 1),
        ("SELECT and(1, 2, 3)", 1),
        ("SELECT and(1, 0, 3)", 0),
        ("SELECT and(-1, 5)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn and_rejects_invalid_argument_counts() {
    for sql in [
        "SELECT and()",
        "SELECT and(1)",
        "SELECT and(1, 'bad')",
        "SELECT and(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
