use rusthouse_query::{Projection, parse_select};

#[test]
fn scalar_bit_logic_evaluates_bounded_constants() {
    for (sql, expected) in [
        ("SELECT BITNAND(0, 0)", -1),
        ("SELECT BITNOR(0, 0)", -1),
        ("SELECT BITXNOR(0, 0)", -1),
        ("SELECT bitnand(7, 3)", -4),
        ("SELECT BITNOR(4, 1)", -6),
        ("SELECT BITXNOR(5, 3)", -7),
        ("SELECT BITXNOR(-1, 0)", 0),
        ("SELECT BITNAND(-1, 15)", -16),
        ("SELECT BITXNOR(BITNAND(0, 0), BITNOR(0, 0))", -1),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn scalar_bit_logic_rejects_invalid_arguments() {
    for sql in [
        "SELECT BITNAND()",
        "SELECT BITNOR(1)",
        "SELECT BITXNOR(1, 2, 3)",
        "SELECT BITNAND(1, 'x')",
        "SELECT BITNOR(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
