use rusthouse_query::{ConstantValue, Projection, parse_select};

#[test]
fn evaluates_case_insensitive_extrema_and_arithmetic() {
    let query =
        parse_select("SELECT GREATEST(1, -5, 3 * 4) AS maximum, least(9, (2 + 3), -2) AS minimum")
            .unwrap();
    match query.projection {
        Projection::Constants(columns) => {
            assert_eq!(columns.len(), 2);
            assert_eq!(columns[0].value, ConstantValue::Integer(12));
            assert_eq!(columns[0].alias.as_deref(), Some("maximum"));
            assert_eq!(columns[1].value, ConstantValue::Integer(-2));
            assert_eq!(columns[1].alias.as_deref(), Some("minimum"));
        }
        other => panic!("unexpected projection: {other:?}"),
    }
    assert_eq!(
        parse_select("SELECT GREATEST(LEAST(20, 9), 2 + 3)")
            .unwrap()
            .projection,
        Projection::Literal(9)
    );
    assert_eq!(
        parse_select("SELECT LEAST(-9223372036854775808, 0)")
            .unwrap()
            .projection,
        Projection::Literal(i64::MIN)
    );
}

#[test]
fn bounds_argument_counts_nesting_and_statement_syntax() {
    let excessive = format!(
        "SELECT GREATEST({})",
        (0..17).map(|v| v.to_string()).collect::<Vec<_>>().join(",")
    );
    for query in [
        "SELECT GREATEST()",
        "SELECT GREATEST(1)",
        "SELECT LEAST(3)",
        "SELECT LEAST(1,)",
        "SELECT GREATEST(1, 2",
        "SELECT GREATEST(1, 2) FROM numbers(4)",
        "SELECT LEAST(1, 9223372036854775808)",
        excessive.as_str(),
    ] {
        assert!(parse_select(query).is_err(), "{query}");
    }
}
