use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn counts_distinct_bytes_including_utf8() {
    for (sql, expected) in [
        ("SELECT stringBytesUniq('Hello')", 4),
        ("SELECT STRINGBYTESUNIQ('AAA')", 1),
        ("SELECT stringBytesUniq('')", 0),
        ("SELECT stringBytesUniq('é')", 2),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT stringBytesUniq(NULL)"), ConstantValue::Null);
}

#[test]
fn rejects_non_string_and_invalid_arity() {
    for sql in [
        "SELECT stringBytesUniq(123)",
        "SELECT stringBytesUniq()",
        "SELECT stringBytesUniq('a', 'b')",
        "SELECT stringBytesUniq('a') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
