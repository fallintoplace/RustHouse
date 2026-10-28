use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(n) => ConstantValue::Integer(n),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_clickhouse_cases() {
    assert_eq!(value("SELECT substringUTF8('hé🙂llo', 2, 2)"), ConstantValue::Text("é🙂".into()));
    assert_eq!(value("SELECT substringUTF8('hé🙂llo', -2)"), ConstantValue::Text("lo".into()));
    assert_eq!(value("SELECT substringUTF8('hé🙂llo', 0)"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT substringUTF8('hé🙂llo', -99, 2)"), ConstantValue::Text("hé".into()));
    assert_eq!(value("SELECT substringUTF8(NULL, 1)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_arguments() {
    for sql in [
        "SELECT substringUTF8('a')",
        "SELECT substringUTF8('a', 1, -2)",
        "SELECT substringUTF8(1, 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
