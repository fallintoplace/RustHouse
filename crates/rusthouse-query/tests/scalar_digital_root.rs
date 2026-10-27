use rusthouse_query::{Projection, parse_select};

#[test]
fn digital_root_handles_positive_negative_and_minimum_int64() {
    for (sql, expected) in [
        ("SELECT DIGITAL_ROOT(0)", 0),
        ("SELECT digital_root(1)", 1),
        ("SELECT DIGITAL_ROOT(9)", 9),
        ("SELECT DIGITAL_ROOT(10)", 1),
        ("SELECT DIGITAL_ROOT(38)", 2),
        ("SELECT DIGITAL_ROOT(-38)", 2),
        ("SELECT DIGITAL_ROOT(999)", 9),
        ("SELECT DIGITAL_ROOT(9223372036854775807)", 7),
        ("SELECT DIGITAL_ROOT(-9223372036854775808)", 8),
        ("SELECT DIGITAL_ROOT(3 * 7) + 1", 4),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn digital_root_rejects_bad_arity_and_unsupported_types() {
    for sql in [
        "SELECT DIGITAL_ROOT()",
        "SELECT DIGITAL_ROOT(1, 2)",
        "SELECT DIGITAL_ROOT(1,)",
        "SELECT DIGITAL_ROOT(1",
        "SELECT DIGITAL_ROOT('bad')",
        "SELECT DIGITAL_ROOT(1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
