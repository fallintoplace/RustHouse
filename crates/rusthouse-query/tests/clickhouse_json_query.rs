use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn json_query_wraps_selected_values() {
    for (sql, expected) in [
        (r#"SELECT JSON_QUERY('{"hello":"world"}', '$.hello')"#, r#"["world"]"#),
        (r#"SELECT JSON_QUERY('{"hello":2}', '$.hello')"#, "[2]"),
        (r#"SELECT JSON_QUERY('{"a":{"x":1}}', '$.a')"#, r#"[{"x":1}]"#),
        (r#"SELECT JSON_QUERY('{"a":[1,2]}', '$.a')"#, "[[1,2]]"),
        (r#"SELECT JSON_QUERY('{"a":[3]}', '$.a[0]')"#, "[3]"),
        (r#"SELECT JSON_QUERY('{"a":1}', '$.missing')"#, ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT JSON_QUERY(NULL, '$.a')"), ConstantValue::Null);
}
#[test]
fn json_query_rejects_unsupported_jsonpaths() {
    for sql in [
        r#"SELECT JSON_QUERY('{"a":1}', 'a')"#,
        r#"SELECT JSON_QUERY('{"a":1}', '$.a[*]')"#,
        "SELECT JSON_QUERY('invalid', '$.x')",
        "SELECT JSON_QUERY('{}', '$.x', '$.y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
