use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn returns_final_path_component() {
    for (sql, expected) in [
        ("SELECT basename('some/long/path/to/file')", "file"),
        ("SELECT basename('file')", "file"),
        ("SELECT basename('/tmp/')", ""),
        ("SELECT basename('')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT basename(NULL)"), ConstantValue::Null);
}
#[test]
fn rejects_invalid_basename_arguments() {
    for sql in [
        "SELECT basename()",
        "SELECT basename(1)",
        "SELECT basename('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
