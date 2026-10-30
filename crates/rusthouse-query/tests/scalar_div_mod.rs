use rusthouse_query::{Projection, parse_select};

#[test]
fn div_mod_handle_signed_values_and_nested_expressions() {
    for (sql, expected) in [
        ("SELECT DIV(7, 3)", 2),
        ("SELECT div(-7, 3)", -2),
        ("SELECT DIV(7, -3)", -2),
        ("SELECT MOD(-7, 3)", -1),
        ("SELECT MOD(7, -3)", 1),
        ("SELECT mod(9, 4) + DIV(7, 3)", 3),
        ("SELECT DIV(-9223372036854775808, 1)", i64::MIN),
        ("SELECT MOD(-9223372036854775808, 3)", -2),
        ("SELECT MOD(-9223372036854775808, -1)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn div_mod_reject_zero_overflow_bad_arity_and_table_inputs() {
    for sql in [
        "SELECT DIV(1, 0)",
        "SELECT MOD(1, 0)",
        "SELECT DIV(-9223372036854775808, -1)",
        "SELECT DIV()",
        "SELECT MOD(1)",
        "SELECT MOD(1, 2, 3)",
        "SELECT DIV(1, 'x')",
        "SELECT DIV(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
