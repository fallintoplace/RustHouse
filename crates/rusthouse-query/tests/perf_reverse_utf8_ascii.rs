use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn ascii_fast_path_retains_unicode_and_null_results() {
    for (sql, expected) in [
        ("SELECT reverseUTF8('abc-123')", ConstantValue::Text("321-cba".into())),
        ("SELECT reverseUTF8('é日a')", ConstantValue::Text("a日é".into())),
        ("SELECT reverse('XYZ')", ConstantValue::Text("ZYX".into())),
        ("SELECT reverseUTF8('')", ConstantValue::Text("".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT reverseUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_bad_argument_counts_and_types() {
    for sql in [
        "SELECT reverseUTF8()",
        "SELECT reverseUTF8('a', 2)",
        "SELECT reverseUTF8(123)",
        "SELECT reverseUTF8('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
