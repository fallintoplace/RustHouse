use rusthouse_query::{Projection, parse_select};

#[test]
fn scalar_round_div_evaluates_signed_and_nested_constants() {
    for (sql, expected) in [
        ("SELECT ROUND_DIV(5, 2)", 3),
        ("SELECT round_div(4, 2)", 2),
        ("SELECT ROUND_DIV(-5, 2)", -3),
        ("SELECT ROUND_DIV(5, -2)", -3),
        ("SELECT ROUND_DIV(-5, -2)", 3),
        ("SELECT ROUND_DIV(1, 3)", 0),
        ("SELECT ROUND_DIV(2 * 5, 4)", 3),
        ("SELECT ROUND_DIV(-9223372036854775808, 1)", i64::MIN),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn scalar_round_div_rejects_invalid_arguments() {
    for sql in [
        "SELECT ROUND_DIV(1, 0)",
        "SELECT ROUND_DIV(-9223372036854775808, -1)",
        "SELECT ROUND_DIV()",
        "SELECT ROUND_DIV(1)",
        "SELECT ROUND_DIV(1, 2, 3)",
        "SELECT ROUND_DIV(1, 'x')",
        "SELECT ROUND_DIV(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
