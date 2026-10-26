use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_checked_left_and_arithmetic_right_bit_shifts() {
    for (sql, expected) in [
        ("SELECT bitShiftLeft(1, 5)", 32),
        ("SELECT BITSHIFTRIGHT(32, 3)", 4),
        ("SELECT bitShiftRight(-8, 2)", -2),
        ("SELECT bitShiftRight(-1, 63)", -1),
        ("SELECT bitShiftLeft(-2, 62)", i64::MIN),
        ("SELECT bitShiftLeft(4, 0)", 4),
        ("SELECT bitShiftRight(8, 1 + 1) + 3", 5),
        ("SELECT bitAnd(bitShiftLeft(3, 2), 15)", 12),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn invalid_shifts_and_arities_fail_closed() {
    for sql in [
        "SELECT bitShiftLeft(1, -1)",
        "SELECT bitShiftRight(1, 64)",
        "SELECT bitShiftLeft(1, 63)",
        "SELECT bitShiftLeft(2, 62)",
        "SELECT bitShiftLeft(-3, 62)",
        "SELECT bitShiftLeft(1)",
        "SELECT bitShiftRight(1, 2, 3)",
        "SELECT bitShiftRight(1,)",
        "SELECT bitShiftLeft(1, 2",
        "SELECT bitShiftLeft(1, 'x')",
        "SELECT bitShiftRight(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
