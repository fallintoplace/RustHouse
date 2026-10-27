use rusthouse_query::{Projection, parse_select};

#[test]
fn bit_rotations_preserve_wrapping_signed_bit_patterns() {
    for (sql, expected) in [
        ("SELECT bitRotateLeft(1, 3)", 8),
        ("SELECT BITROTATERIGHT(8, 3)", 1),
        ("SELECT bitRotateLeft(1, 63)", i64::MIN),
        ("SELECT bitRotateRight(1, 1)", i64::MIN),
        ("SELECT bitRotateLeft(-9223372036854775808, 1)", 1),
        ("SELECT bitRotateRight(-1, 12)", -1),
        ("SELECT bitRotateLeft(7, 0)", 7),
        ("SELECT bitRotateRight(bitRotateLeft(123456, 7), 7)", 123456),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn bit_rotations_reject_invalid_shifts_and_arity() {
    for sql in [
        "SELECT bitRotateLeft(1, -1)",
        "SELECT bitRotateRight(1, 64)",
        "SELECT bitRotateLeft(1)",
        "SELECT bitRotateRight(1, 2, 3)",
        "SELECT bitRotateRight(1,)",
        "SELECT bitRotateLeft(1, 2",
        "SELECT bitRotateLeft(1, '2')",
        "SELECT bitRotateRight(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
