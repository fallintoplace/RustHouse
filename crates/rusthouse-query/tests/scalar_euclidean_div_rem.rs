use rusthouse_query::{Projection, parse_select};

#[test]
fn scalar_euclidean_div_rem_evaluates_signed_and_nested_constants() {
    for (sql, expected) in [
        ("SELECT DIV_EUCLID(-7, 3)", -3),
        ("SELECT REM_EUCLID(-7, 3)", 2),
        ("SELECT DIV_EUCLID(-7, -3)", 3),
        ("SELECT DIV_EUCLID(7, -3)", -2),
        ("SELECT REM_EUCLID(7, -3)", 1),
        ("SELECT div_euclid(8, 3) + REM_EUCLID(8, 3)", 4),
        ("SELECT REM_EUCLID(-9223372036854775808, 3)", 1),
        ("SELECT REM_EUCLID(-9223372036854775808, -1)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn scalar_euclidean_div_rem_rejects_invalid_arguments() {
    for sql in [
        "SELECT DIV_EUCLID(1, 0)",
        "SELECT REM_EUCLID(1, 0)",
        "SELECT DIV_EUCLID(-9223372036854775808, -1)",
        "SELECT DIV_EUCLID(1)",
        "SELECT REM_EUCLID(1, 2, 3)",
        "SELECT DIV_EUCLID('x', 2)",
        "SELECT REM_EUCLID(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
