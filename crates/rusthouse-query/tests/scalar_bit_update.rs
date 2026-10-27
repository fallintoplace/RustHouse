use rusthouse_query::{Projection, parse_select};

#[test]
fn bit_update_preserves_twos_complement_and_high_bit() {
    for (sql, expected) in [
        ("SELECT BIT_SET(0, 0)", 1),
        ("SELECT bit_set(1, 2)", 5),
        ("SELECT BIT_CLEAR(7, 1)", 5),
        ("SELECT BIT_CLEAR(0, 5)", 0),
        ("SELECT BIT_TOGGLE(5, 0)", 4),
        ("SELECT BIT_TOGGLE(4, 0)", 5),
        ("SELECT BIT_SET(0, 63)", i64::MIN),
        ("SELECT BIT_CLEAR(-1, 63)", i64::MAX),
        ("SELECT BIT_TOGGLE(-9223372036854775808, 63)", 0),
        ("SELECT BIT_SET(-1, 63)", -1),
        ("SELECT BIT_CLEAR(BIT_SET(0, 3), 3)", 0),
        ("SELECT BIT_TOGGLE(2 + 3, 1 + 1)", 1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn bit_update_rejects_out_of_range_indexes_and_bad_calls() {
    for sql in [
        "SELECT BIT_SET(1, -1)",
        "SELECT BIT_CLEAR(1, 64)",
        "SELECT BIT_TOGGLE(1, 9223372036854775807)",
        "SELECT BIT_SET()",
        "SELECT BIT_CLEAR(1)",
        "SELECT BIT_TOGGLE(1, 2, 3)",
        "SELECT BIT_SET(1,)",
        "SELECT BIT_CLEAR(1, 2",
        "SELECT BIT_TOGGLE(1, 'x')",
        "SELECT BIT_SET(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
