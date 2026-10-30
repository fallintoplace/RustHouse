use rusthouse_query::{Projection, parse_select};

#[test]
fn or_folds_boolean_constant_arguments() {
    for (sql, expected) in [
        ("SELECT or(0, 0)", 0),
        ("SELECT or(0, 0, 0)", 0),
        ("SELECT or(0, -1, 0)", 1),
        ("SELECT or(1, 0)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn or_rejects_invalid_argument_counts() {
    for sql in [
        "SELECT or()",
        "SELECT or(1)",
        "SELECT or(1, 'bad')",
        "SELECT or(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
