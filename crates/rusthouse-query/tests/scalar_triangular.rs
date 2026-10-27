use rusthouse_query::{Projection, parse_select};

#[test]
fn triangular_numbers_handle_zero_large_inputs_and_composition() {
    for (sql, expected) in [
        ("SELECT TRIANGULAR(0)", 0),
        ("SELECT triangular(1)", 1),
        ("SELECT TRIANGULAR(2)", 3),
        ("SELECT TRIANGULAR(7)", 28),
        ("SELECT TRIANGULAR(10 + 1)", 66),
        ("SELECT TRIANGULAR(4294967295)", 9_223_372_034_707_292_160),
        ("SELECT TRIANGULAR(3) + TRIANGULAR(4)", 16),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn triangular_rejects_negative_overflowing_and_malformed_calls() {
    for sql in [
        "SELECT TRIANGULAR(-1)",
        "SELECT TRIANGULAR(-9223372036854775808)",
        "SELECT TRIANGULAR(4294967296)",
        "SELECT TRIANGULAR(9223372036854775807)",
        "SELECT TRIANGULAR()",
        "SELECT TRIANGULAR(1, 2)",
        "SELECT TRIANGULAR(1,)",
        "SELECT TRIANGULAR(1",
        "SELECT TRIANGULAR('x')",
        "SELECT TRIANGULAR(1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
