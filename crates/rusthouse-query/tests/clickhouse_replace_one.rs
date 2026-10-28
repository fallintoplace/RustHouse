use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn clickhouse_replace_one_supports_values_and_null() {
    for (sql, expected) in [
        ("SELECT replaceOne('banana', 'na', '_')", ConstantValue::Text("ba_na".into())),
        ("SELECT replaceOne('foo', 'x', 'y')", ConstantValue::Text("foo".into())),
        ("SELECT replaceOne('abc','', 'Z')", ConstantValue::Text("abc".into())),
        ("SELECT replaceOne(NULL, 'a', 'b')", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn clickhouse_replace_one_rejects_wrong_arity_and_types() {
    for sql in [
        "SELECT replaceOne('a','b')",
        "SELECT replaceOne('a','b','c','d')",
        "SELECT replaceOne('a', 1, 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
