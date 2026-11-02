use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn concat_with_separator_and_alias_propagate_null() {
    for (query, expected) in [
        ("SELECT concatWithSeparator('-', 'a', 'b')", ConstantValue::Text("a-b".into())),
        ("SELECT concatWithSeparator('-', 'a', NULL)", ConstantValue::Null),
        ("SELECT concatWithSeparator('-', NULL, 'b')", ConstantValue::Null),
        ("SELECT concatWithSeparator(NULL, 'a', 'b')", ConstantValue::Null),
        ("SELECT concat_ws('-', 'a', NULL, 'b')", ConstantValue::Null),
    ] {
        assert_eq!(value(query), expected, "{query}");
    }
    assert!(parse_select("SELECT concatWithSeparator('-', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q')").is_err());
}
