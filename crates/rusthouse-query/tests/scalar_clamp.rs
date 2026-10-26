use rusthouse_query::{Projection, parse_select};

#[test]
fn clamp_bounds_integer_constant_expressions() {
    for (sql, expected) in [
        ("SELECT CLAMP(12, 0, 10)", 10),
        ("SELECT clamp(-3, -2, 4)", -2),
        ("SELECT Clamp(3, 3, 3)", 3),
        ("SELECT 2 + CLAMP(2 * 3, 1, 10)", 8),
        ("SELECT CLAMP(-9223372036854775808, -9223372036854775808, 0)", i64::MIN),
        ("SELECT GREATEST(CLAMP(12, 0, 10), 11)", 11),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn clamp_rejects_reversed_bounds_missing_and_extra_arguments() {
    for sql in [
        "SELECT CLAMP()",
        "SELECT CLAMP(1)",
        "SELECT CLAMP(1, 2)",
        "SELECT CLAMP(1, 2, 3, 4)",
        "SELECT CLAMP(1, 3, 2)",
        "SELECT CLAMP(1, 2,)",
        "SELECT CLAMP(1, 2, 3",
        "SELECT CLAMP(1, 'bad', 3)",
        "SELECT CLAMP(1, 0, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
