use rusthouse_query::{ConstantValue, Projection, parse_select};

#[test]
fn rounded_division_handles_signs_and_exact_quotients() {
    for (sql, expected) in [
        ("SELECT CEIL_DIV(7, 3)", 3),
        ("SELECT FLOOR_DIV(7, 3)", 2),
        ("SELECT ceil_div(-7, 3)", -2),
        ("SELECT floor_div(-7, 3)", -3),
        ("SELECT CEIL_DIV(7, -3)", -2),
        ("SELECT FLOOR_DIV(7, -3)", -3),
        ("SELECT CEIL_DIV(-7, -3)", 3),
        ("SELECT FLOOR_DIV(-7, -3)", 2),
        ("SELECT CEIL_DIV(6, -3)", -2),
        ("SELECT FLOOR_DIV(-6, 3)", -2),
        ("SELECT CEIL_DIV(0, -3)", 0),
        (
            "SELECT FLOOR_DIV(-9223372036854775808, 3)",
            -3074457345618258603,
        ),
        (
            "SELECT CEIL_DIV(-9223372036854775808, 3)",
            -3074457345618258602,
        ),
        ("SELECT CEIL_DIV(1 + 4, 2) * 3", 9),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
    let query = parse_select("SELECT ceil_div(9, 4) AS pages, floor_div(9, 4) AS batches").unwrap();
    let Projection::Constants(columns) = query.projection else {
        panic!("expected two constant expressions");
    };
    assert_eq!(columns[0].value, ConstantValue::Integer(3));
    assert_eq!(columns[1].value, ConstantValue::Integer(2));
}

#[test]
fn rounded_division_rejects_zero_overflow_and_bad_arguments() {
    for sql in [
        "SELECT CEIL_DIV(1, 0)",
        "SELECT FLOOR_DIV(1, 0)",
        "SELECT CEIL_DIV(-9223372036854775808, -1)",
        "SELECT FLOOR_DIV(-9223372036854775808, -1)",
        "SELECT CEIL_DIV(1)",
        "SELECT CEIL_DIV()",
        "SELECT FLOOR_DIV(1, 2, 3)",
        "SELECT FLOOR_DIV(1,)",
        "SELECT FLOOR_DIV(1, 2",
        "SELECT CEIL_DIV(1, 'abc')",
        "SELECT FLOOR_DIV(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
