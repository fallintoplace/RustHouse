use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn matches_complete_tokens_not_substrings() {
    for (sql, expected) in [
        ("SELECT hasToken('clickhouse test', 'test')", 1),
        ("SELECT hasToken('some_thing 12', 'some_thing')", 1),
        ("SELECT hasToken('testing test2', 'test')", 0),
        ("SELECT hasToken('Hello world', 'hello')", 0),
        ("SELECT hasToken('a,b!', 'b')", 1),
    ] { assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}"); }
    assert_eq!(value("SELECT hasToken(NULL, 'a')"), ConstantValue::Null);
}
#[test]
fn rejects_ill_formed_tokens() {
    for sql in [
        "SELECT hasToken('hi', '')",
        "SELECT hasToken('hi', 'two words')",
        "SELECT hasToken('hi', 'a-b')",
        "SELECT hasToken(42, 'hi')",
        "SELECT hasToken('hi')",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
