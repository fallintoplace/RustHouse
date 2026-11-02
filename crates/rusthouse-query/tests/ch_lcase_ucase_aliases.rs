use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn lcase_ucase_use_ascii_only_case_conversion() {
    for (query, expected) in [
        ("SELECT lcase('HELLO É')", ConstantValue::Text("hello É".into())),
        ("SELECT UCASE('hello é')", ConstantValue::Text("HELLO é".into())),
        ("SELECT lcase(NULL)", ConstantValue::Null),
        ("SELECT ucase(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(query), expected, "{query}");
    }
    assert!(parse_select("SELECT lcase('abc', 'x')").is_err());
    assert!(parse_select("SELECT ucase()").is_err());
}
