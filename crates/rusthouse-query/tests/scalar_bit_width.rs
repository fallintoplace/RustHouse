use rusthouse_query::{Projection, parse_select};

#[test]
fn bit_width_counts_significant_magnitude_bits() {
    for (sql, expected) in [
        ("SELECT BIT_WIDTH(0)", 0),
        ("SELECT bit_width(1)", 1),
        ("SELECT BIT_WIDTH(2)", 2),
        ("SELECT BIT_WIDTH(3)", 2),
        ("SELECT BIT_WIDTH(-255)", 8),
        ("SELECT BIT_WIDTH(256)", 9),
        ("SELECT BIT_WIDTH(9223372036854775807)", 63),
        ("SELECT BIT_WIDTH(-9223372036854775808)", 64),
        ("SELECT BIT_WIDTH(1 + 7)", 4),
        ("SELECT GREATEST(BIT_WIDTH(255), 3)", 8),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn bit_width_rejects_invalid_argument_lists_and_sources() {
    for sql in [
        "SELECT BIT_WIDTH()",
        "SELECT BIT_WIDTH(1, 2)",
        "SELECT BIT_WIDTH(1",
        "SELECT BIT_WIDTH('bad')",
        "SELECT BIT_WIDTH(1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
