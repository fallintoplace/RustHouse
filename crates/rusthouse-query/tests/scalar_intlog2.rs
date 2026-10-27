use rusthouse_query::{Projection, parse_select};

#[test]
fn intlog2_floors_logarithms_without_floating_point() {
    for (sql, expected) in [
        ("SELECT INTLOG2(1)", 0),
        ("SELECT intLog2(2)", 1),
        ("SELECT IntLoG2(3)", 1),
        ("SELECT INTLOG2(8)", 3),
        ("SELECT INTLOG2(9)", 3),
        ("SELECT INTLOG2(9223372036854775807)", 62),
        ("SELECT INTLOG2(POWER(2, 10)) + 1", 11),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn intlog2_rejects_zero_negatives_and_bad_arity() {
    for sql in [
        "SELECT INTLOG2(0)",
        "SELECT INTLOG2(-1)",
        "SELECT INTLOG2(-9223372036854775808)",
        "SELECT INTLOG2()",
        "SELECT INTLOG2(2, 3)",
        "SELECT INTLOG2(8",
        "SELECT INTLOG2('8')",
        "SELECT INTLOG2(8) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
