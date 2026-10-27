use rusthouse_query::{Projection, parse_select};

#[test]
fn scalar_floor_multiple_evaluates_signed_and_nested_constants() {
    for (sql, expected) in [
        ("SELECT FLOOR_TO_MULTIPLE(14, 5)", 10),
        ("SELECT floor_to_multiple(-14, 5)", -15),
        ("SELECT FLOOR_TO_MULTIPLE(-15, 5)", -15),
        ("SELECT FLOOR_TO_MULTIPLE(0, 5)", 0),
        ("SELECT FLOOR_TO_MULTIPLE(3 * 7, 4)", 20),
        ("SELECT FLOOR_TO_MULTIPLE(9223372036854775807, 2)", 9223372036854775806),
        ("SELECT FLOOR_TO_MULTIPLE(-9223372036854775808, 2)", i64::MIN),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn scalar_floor_multiple_rejects_invalid_arguments() {
    for sql in [
        "SELECT FLOOR_TO_MULTIPLE(1, 0)",
        "SELECT FLOOR_TO_MULTIPLE(1, -2)",
        "SELECT FLOOR_TO_MULTIPLE(-9223372036854775808, 3)",
        "SELECT FLOOR_TO_MULTIPLE(1)",
        "SELECT FLOOR_TO_MULTIPLE(1, 2, 3)",
        "SELECT FLOOR_TO_MULTIPLE(1, 'x')",
        "SELECT FLOOR_TO_MULTIPLE(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
