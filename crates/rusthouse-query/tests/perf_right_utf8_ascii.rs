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
        ("SELECT rightUTF8('abcdef', 3)", ConstantValue::Text("def".into())),
        ("SELECT rightUTF8('éabc', 2)", ConstantValue::Text("bc".into())),
        ("SELECT rightUTF8('abé', 2)", ConstantValue::Text("bé".into())),
        ("SELECT rightUTF8('é', 50)", ConstantValue::Text("é".into())),
        ("SELECT rightUTF8('abc', 0)", ConstantValue::Text("".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT rightUTF8(NULL, 2)"), ConstantValue::Null);
}

#[test]
fn rejects_bad_argument_counts_and_types() {
    for sql in [
        "SELECT rightUTF8()",
        "SELECT rightUTF8('a', 2, 3)",
        "SELECT rightUTF8(123, 2)",
        "SELECT rightUTF8('a', -1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
