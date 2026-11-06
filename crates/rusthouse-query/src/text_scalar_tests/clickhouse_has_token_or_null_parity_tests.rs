use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn matches_tokens_and_returns_null_for_invalid_needles() {
    assert_eq!(
        value("SELECT hasTokenOrNull('hello, clickhouse!', 'clickhouse')"),
        ConstantValue::Integer(1),
    );
    assert_eq!(
        value("SELECT hasTokenOrNull('hello, clickhouse!', 'click')"),
        ConstantValue::Integer(0),
    );
    assert_eq!(
        value("SELECT hasTokenOrNull('foo_bar', 'foo_bar')"),
        ConstantValue::Integer(1),
    );
    for query in [
        "SELECT hasTokenOrNull('abc', '!!!')",
        "SELECT hasTokenOrNull('abc', 'a b')",
        "SELECT hasTokenOrNull('abc', '')",
        "SELECT hasTokenOrNull(NULL, 'abc')",
        "SELECT hasTokenOrNull('abc', NULL)",
    ] {
        assert_eq!(value(query), ConstantValue::Null, "{query}");
    }
    assert!(parse_select("SELECT hasTokenOrNull('a')").is_err());
    assert!(parse_select("SELECT hasToken('a', '!!!')").is_err());
}
