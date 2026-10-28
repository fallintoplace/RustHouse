use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn clickhouse_constant_function() {
    for (sql, expected) in [
        ("SELECT rightPadUTF8('猫', 3, '~')", ConstantValue::Text("猫~~".into())),
        ("SELECT rightPadUTF8('abc', 2)", ConstantValue::Text("ab".into())),
        ("SELECT rightPadUTF8(NULL, 2)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn invalid_arguments() {
    for sql in [
        "SELECT rightPadUTF8()",
        "SELECT rightPadUTF8('a')",
        "SELECT rightPadUTF8('a', -1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
