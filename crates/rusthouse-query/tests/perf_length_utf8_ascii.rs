use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn ascii_fast_path_matches_unicode_and_null_behavior() {
    for (sql, expected) in [
        ("SELECT lengthUTF8('abcdef')", ConstantValue::Integer(6)),
        ("SELECT lengthUTF8('')", ConstantValue::Integer(0)),
        ("SELECT lengthUTF8('München')", ConstantValue::Integer(7)),
        ("SELECT lengthUTF8('🍁é')", ConstantValue::Integer(2)),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT lengthUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_invalid_utf8_function_arguments() {
    for sql in [
        "SELECT lengthUTF8()",
        "SELECT lengthUTF8('a', 'b')",
        "SELECT lengthUTF8(123)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
