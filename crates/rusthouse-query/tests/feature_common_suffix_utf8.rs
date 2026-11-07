use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn supports_common_suffix_utf8() {
    for (sql, expected) in [
        ("SELECT commonSuffixLengthUTF8('x🦀é', 'y🦀é')", ConstantValue::Integer(2)),
        ("SELECT commonSuffixLengthUTF8('one', 'two')", ConstantValue::Integer(0)),
        ("SELECT commonSuffixLengthUTF8('a', '')", ConstantValue::Integer(0)),
        ("SELECT commonSuffixLengthUTF8('x', NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_invalid_common_suffix_utf8_arity() {
    for sql in ["SELECT commonSuffixLengthUTF8('x')", "SELECT commonSuffixLengthUTF8('x','y','z')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
