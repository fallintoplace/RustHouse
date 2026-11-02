use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_quoted_keys_with_dots_spaces_and_utf8() {
    for (sql, expected) in [
        (r#"SELECT JSON_EXISTS('{"order.id":{"line items":[7,8]}}', '$[''order.id''][''line items''][1]')"#, 1),
        (r#"SELECT JSON_EXISTS('{"order.id":{"line items":[7,8]}}', '$[''order.id''][''line items''][2]')"#, 0),
        (r#"SELECT JSON_EXISTS('{"cafés":2}', '$[''cafés'']')"#, 1),
        (r#"SELECT JSON_EXISTS('{"":null}', '$['''']')"#, 1),
        (r#"SELECT JSON_EXISTS('{"0":42}', '$[''0'']')"#, 1),
        (r#"SELECT JSON_EXISTS('[42]', '$[''0'']')"#, 0),
        (r#"SELECT JSON_EXISTS('{"a":{"b":1}}', '$.a[''b'']')"#, 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
}

#[test]
fn rejects_bad_quoted_path_syntax_instead_of_misselecting_fields() {
    for sql in [
        r#"SELECT JSON_EXISTS('{"a":1}', '$[''a'')"#,
        r#"SELECT JSON_EXISTS('{"a":1}', '$[''a'']garbage')"#,
        r#"SELECT JSON_EXISTS('{"a":1}', '$[''a\\b'']')"#,
        r#"SELECT JSON_EXISTS('{"a":1}', '$[''a''b'']')"#,
        r#"SELECT JSON_EXISTS('[42]', '$[01]x')"#,
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
