use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn validates_json_with_clickhouse_style_function() {
    assert_eq!(
        value(r#"SELECT isValidJSON('{"a":[1,true,null]}')"#),
        ConstantValue::Integer(1)
    );
    assert_eq!(
        value(r#"SELECT isValidJSON('{"a":01}')"#),
        ConstantValue::Integer(0)
    );
    assert_eq!(value("SELECT isValidJSON(NULL)"), ConstantValue::Null);
}

#[test]
fn extracts_string_raw_and_presence_from_nested_json() {
    let json = r#"{"user":{"name":"Ada","score":12},"items":[{"id":3}]}"#;
    assert_eq!(
        value(&format!("SELECT JSONExtractString('{json}', 'user.name')")),
        ConstantValue::Text("Ada".into())
    );
    assert_eq!(
        value(&format!("SELECT JSONExtractRaw('{json}', 'user.score')")),
        ConstantValue::Text("12".into())
    );
    assert_eq!(
        value(&format!("SELECT JSONHas('{json}', 'items.0.id')")),
        ConstantValue::Integer(1)
    );
    assert_eq!(
        value(&format!("SELECT JSONHas('{json}', 'missing')")),
        ConstantValue::Integer(0)
    );
    assert_eq!(
        value(&format!("SELECT JSONExtractString('{json}', 'missing')")),
        ConstantValue::Text(String::new())
    );
    assert_eq!(
        value(&format!("SELECT JSONExtractString('{json}', 'user.score')")),
        ConstantValue::Text(String::new())
    );
}

#[test]
fn rejects_invalid_json_and_argument_shapes() {
    for sql in [
        "SELECT JSONExtractString('{', 'a')",
        "SELECT JSONHas('[]')",
        "SELECT JSONExtractRaw('{}', 'a', 'b')",
        "SELECT isValidJSON(123)",
        "SELECT JSONExtractString('{}', 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
