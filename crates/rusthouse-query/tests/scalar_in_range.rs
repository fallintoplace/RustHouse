use rusthouse_query::{Projection, parse_select};

#[test]
fn in_range_checks_inclusive_signed_boundaries_and_nested_expressions() {
    for (sql, expected) in [
        ("SELECT IN_RANGE(5, 1, 10)", 1),
        ("SELECT in_range(1, 1, 10)", 1),
        ("SELECT IN_RANGE(10, 1, 10)", 1),
        ("SELECT IN_RANGE(11, 1, 10)", 0),
        ("SELECT IN_RANGE(-5, -10, -1)", 1),
        ("SELECT IN_RANGE(0, 1, 10)", 0),
        (
            "SELECT IN_RANGE(-9223372036854775808, -9223372036854775808, 0)",
            1,
        ),
        ("SELECT IN_RANGE(2 * 3, 2, 3 * 2) + 4", 5),
        ("SELECT IN_RANGE(CLAMP(7, 0, 5), 1, 5)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn in_range_rejects_bad_bounds_and_malformed_calls() {
    for sql in [
        "SELECT IN_RANGE()",
        "SELECT IN_RANGE(1)",
        "SELECT IN_RANGE(1, 2)",
        "SELECT IN_RANGE(1, 3, 2)",
        "SELECT IN_RANGE(1, 0, 2, 3)",
        "SELECT IN_RANGE(1, 0,)",
        "SELECT IN_RANGE(1, 0, 2",
        "SELECT IN_RANGE('x', 0, 1)",
        "SELECT IN_RANGE(1, 0, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
