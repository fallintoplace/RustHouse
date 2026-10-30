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
        ("SELECT leftUTF8('abcdef', 3)", ConstantValue::Text("abc".into())),
        ("SELECT leftUTF8('éabc', 2)", ConstantValue::Text("éa".into())),
        ("SELECT leftUTF8('abcé', 3)", ConstantValue::Text("abc".into())),
        ("SELECT leftUTF8('é', 50)", ConstantValue::Text("é".into())),
        ("SELECT leftUTF8('abc', 0)", ConstantValue::Text("".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT leftUTF8(NULL, 2)"), ConstantValue::Null);
}

#[test]
fn rejects_bad_argument_counts_and_types() {
    for sql in [
        "SELECT leftUTF8()",
        "SELECT leftUTF8('a', 2, 3)",
        "SELECT leftUTF8(123, 2)",
        "SELECT leftUTF8('a', -1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
