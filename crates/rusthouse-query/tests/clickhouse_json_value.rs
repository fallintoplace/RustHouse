use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn json_value_extracts_exact_scalar_paths() {
    for (sql, expected) in [
        (r#"SELECT JSON_VALUE('{"name":"Ada"}', '$.name')"#, "Ada"),
        (r#"SELECT JSON_VALUE('{"a":[{"x":7}]}', '$.a[0].x')"#, "7"),
        (r#"SELECT JSON_VALUE('{"enabled":true}', '$.enabled')"#, "true"),
        (r#"SELECT JSON_VALUE('{"a":[1,2]}', '$.a')"#, ""),
        (r#"SELECT JSON_VALUE('{"a":null}', '$.a')"#, ""),
        (r#"SELECT JSON_VALUE('{"a":1}', '$.missing')"#, ""),
        (r#"SELECT JSON_VALUE('"text"', '$')"#, "text"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT JSON_VALUE(NULL, '$.a')"), ConstantValue::Null);
}

#[test]
fn json_value_rejects_unsupported_jsonpaths_and_invalid_arguments() {
    for sql in [
        r#"SELECT JSON_VALUE('{"a":1}', 'a')"#,
        r#"SELECT JSON_VALUE('{"a":1}', '$.a[*]')"#,
        r#"SELECT JSON_VALUE('{"a":1}', '$..a')"#,
        r#"SELECT JSON_VALUE('{"a":1}', '$.a[foo]')"#,
        "SELECT JSON_VALUE('bad', '$')",
        "SELECT JSON_VALUE('{}', 1)",
        "SELECT JSON_VALUE('{}', '$', '$')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
