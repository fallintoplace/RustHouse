use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn ends_with_utf8_values() {
    for (sql, expected) in [
        ("SELECT endsWithUTF8('привет','вет')", ConstantValue::Integer(1)),
        ("SELECT endsWithUTF8('привет','при')", ConstantValue::Integer(0)),
        ("SELECT endsWithUTF8('abc','')", ConstantValue::Integer(1)),
        ("SELECT endsWithUTF8(NULL,'x')", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn ends_with_utf8_invalid_calls() {
    for sql in [
        "SELECT endsWithUTF8()",
        "SELECT endsWithUTF8('x')",
        "SELECT endsWithUTF8('x','y','z')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
