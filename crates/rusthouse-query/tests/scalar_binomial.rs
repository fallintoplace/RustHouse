use rusthouse_query::{Projection, parse_select};

#[test]
fn binomial_computes_exact_combinations_with_symmetry() {
    for (sql, expected) in [
        ("SELECT BINOMIAL(0, 0)", 1),
        ("SELECT binomial(5, 0)", 1),
        ("SELECT BINOMIAL(5, 5)", 1),
        ("SELECT BINOMIAL(5, 2)", 10),
        ("SELECT BINOMIAL(5, 3)", 10),
        ("SELECT BINOMIAL(6, 3)", 20),
        ("SELECT BINOMIAL(66, 1)", 66),
        ("SELECT BINOMIAL(66, 65)", 66),
        ("SELECT BINOMIAL(66, 33)", 7_219_428_434_016_265_740),
        ("SELECT BINOMIAL(2 + 3, 2) + 1", 11),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn binomial_rejects_invalid_limits_and_malformed_arguments() {
    for sql in [
        "SELECT BINOMIAL(-1, 0)",
        "SELECT BINOMIAL(5, -1)",
        "SELECT BINOMIAL(5, 6)",
        "SELECT BINOMIAL(67, 1)",
        "SELECT BINOMIAL()",
        "SELECT BINOMIAL(1)",
        "SELECT BINOMIAL(1, 0, 0)",
        "SELECT BINOMIAL(1,)",
        "SELECT BINOMIAL(1, 0",
        "SELECT BINOMIAL('x', 0)",
        "SELECT BINOMIAL(3, 1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
