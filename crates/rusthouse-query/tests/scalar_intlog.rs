use rusthouse_query::{Projection, parse_select};

#[test]
fn scalar_intlog_supports_signed_and_composed_constants() {
    for (sql, expected) in [
        ("SELECT INTLOG(1, 2)", 0),
        ("SELECT intlog(8, 2)", 3),
        ("SELECT INTLOG(9, 2)", 3),
        ("SELECT INTLOG(999, 10)", 2),
        ("SELECT INTLOG(1000, 10)", 3),
        ("SELECT INTLOG(9223372036854775807, 2)", 62),
        ("SELECT INTLOG(3 * 9, 3) + 1", 4),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn scalar_intlog_rejects_invalid_arguments() {
    for sql in [
        "SELECT INTLOG(0, 2)",
        "SELECT INTLOG(-1, 2)",
        "SELECT INTLOG(10, 1)",
        "SELECT INTLOG(10, 0)",
        "SELECT INTLOG(10, -2)",
        "SELECT INTLOG(10)",
        "SELECT INTLOG(10, 2, 3)",
        "SELECT INTLOG('x', 2)",
        "SELECT INTLOG(10, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
