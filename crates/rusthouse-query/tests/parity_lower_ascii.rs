use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn clickhouse_compatibility_cases() {
    for (sql, expected) in [
        ("SELECT lower('ABCäÖ')", ConstantValue::Text("abcäÖ".into())),
        ("SELECT lower('LOUD')", ConstantValue::Text("loud".into())),
        ("SELECT lower('ÀBß')", ConstantValue::Text("Àbß".into())),
        ("SELECT lowerUTF8('ABCäÖ')", ConstantValue::Text("abcäö".into())),
        ("SELECT lower(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_arguments() {
    let sql = "SELECT lower('a', 'b')";
    assert!(parse_select(sql).is_err(), "{sql}");
}
