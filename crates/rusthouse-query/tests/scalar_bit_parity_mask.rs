use rusthouse_query::{Projection, parse_select};

#[test]
fn scalar_bit_parity_mask_evaluates_bounded_constants() {
    for (sql, expected) in [
        ("SELECT BIT_PARITY(0)", 0),
        ("SELECT bit_parity(1)", 1),
        ("SELECT BIT_PARITY(3)", 0),
        ("SELECT BIT_PARITY(-1)", 0),
        ("SELECT BIT_PARITY(-9223372036854775808)", 1),
        ("SELECT BIT_MASK(0)", 0),
        ("SELECT BIT_MASK(1)", 1),
        ("SELECT BIT_MASK(3)", 7),
        ("SELECT BIT_MASK(63)", i64::MAX),
        ("SELECT BIT_MASK(64)", -1),
        ("SELECT BIT_PARITY(BIT_MASK(4))", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn scalar_bit_parity_mask_rejects_invalid_arguments() {
    for sql in [
        "SELECT BIT_MASK(-1)",
        "SELECT BIT_MASK(65)",
        "SELECT BIT_MASK()",
        "SELECT BIT_PARITY()",
        "SELECT BIT_PARITY(1, 2)",
        "SELECT BIT_MASK('x')",
        "SELECT BIT_PARITY(1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
