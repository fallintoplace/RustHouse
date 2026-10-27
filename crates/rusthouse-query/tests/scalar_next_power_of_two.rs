use rusthouse_query::{Projection, parse_select};

#[test]
fn next_power_of_two_rounds_nonnegative_integers_up() {
    for (sql, expected) in [
        ("SELECT NEXT_POWER_OF_TWO(0)", 1),
        ("SELECT next_power_of_two(1)", 1),
        ("SELECT NEXT_POWER_OF_TWO(2)", 2),
        ("SELECT NEXT_POWER_OF_TWO(3)", 4),
        ("SELECT NEXT_POWER_OF_TWO(9)", 16),
        ("SELECT NEXT_POWER_OF_TWO(1023)", 1024),
        (
            "SELECT NEXT_POWER_OF_TWO(4611686018427387904)",
            4611686018427387904,
        ),
        ("SELECT NEXT_POWER_OF_TWO(3 * 5)", 16),
        ("SELECT NEXT_POWER_OF_TWO(BITSHIFTLEFT(1, 10) + 1)", 2048),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn next_power_of_two_rejects_negative_and_overflowing_values() {
    for sql in [
        "SELECT NEXT_POWER_OF_TWO(-1)",
        "SELECT NEXT_POWER_OF_TWO(-9223372036854775808)",
        "SELECT NEXT_POWER_OF_TWO(4611686018427387905)",
        "SELECT NEXT_POWER_OF_TWO(9223372036854775807)",
        "SELECT NEXT_POWER_OF_TWO()",
        "SELECT NEXT_POWER_OF_TWO(1, 2)",
        "SELECT NEXT_POWER_OF_TWO(1",
        "SELECT NEXT_POWER_OF_TWO('bad')",
        "SELECT NEXT_POWER_OF_TWO(1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
