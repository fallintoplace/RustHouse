use rusthouse_query::{Projection, parse_select};

#[test]
fn is_power_of_two_returns_integer_boolean_for_positive_powers() {
    for (sql, expected) in [
        ("SELECT IS_POWER_OF_TWO(0)", 0),
        ("SELECT is_power_of_two(1)", 1),
        ("SELECT IS_POWER_OF_TWO(2)", 1),
        ("SELECT IS_POWER_OF_TWO(3)", 0),
        ("SELECT IS_POWER_OF_TWO(-8)", 0),
        ("SELECT IS_POWER_OF_TWO(-9223372036854775808)", 0),
        ("SELECT IS_POWER_OF_TWO(4611686018427387904)", 1),
        ("SELECT IS_POWER_OF_TWO(9223372036854775807)", 0),
        ("SELECT IS_POWER_OF_TWO(1 + 7)", 1),
        ("SELECT IS_POWER_OF_TWO(POWER(2, 12))", 1),
        ("SELECT 10 + IS_POWER_OF_TWO(9)", 10),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn is_power_of_two_rejects_bad_arity_and_sources() {
    for sql in [
        "SELECT IS_POWER_OF_TWO()",
        "SELECT IS_POWER_OF_TWO(1, 2)",
        "SELECT IS_POWER_OF_TWO(1",
        "SELECT IS_POWER_OF_TWO('bad')",
        "SELECT IS_POWER_OF_TWO(1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
