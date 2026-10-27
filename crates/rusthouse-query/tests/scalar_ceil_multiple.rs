use rusthouse_query::{Projection, parse_select};

#[test]
fn scalar_ceil_multiple_supports_signed_and_composed_constants() {
    for (sql, expected) in [
        ("SELECT CEIL_TO_MULTIPLE(14, 5)", 15),
        ("SELECT ceil_to_multiple(-14, 5)", -10),
        ("SELECT CEIL_TO_MULTIPLE(-15, 5)", -15),
        ("SELECT CEIL_TO_MULTIPLE(0, 5)", 0),
        ("SELECT CEIL_TO_MULTIPLE(2 + 7, 4)", 12),
        ("SELECT CEIL_TO_MULTIPLE(-9223372036854775808, 2)", i64::MIN),
        ("SELECT CEIL_TO_MULTIPLE(-9223372036854775808, 3)", -9223372036854775806),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn scalar_ceil_multiple_rejects_invalid_arguments() {
    for sql in [
        "SELECT CEIL_TO_MULTIPLE(1, 0)",
        "SELECT CEIL_TO_MULTIPLE(1, -2)",
        "SELECT CEIL_TO_MULTIPLE(9223372036854775807, 2)",
        "SELECT CEIL_TO_MULTIPLE(1)",
        "SELECT CEIL_TO_MULTIPLE(1, 2, 3)",
        "SELECT CEIL_TO_MULTIPLE(1, 'x')",
        "SELECT CEIL_TO_MULTIPLE(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
