use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn port_requires_brackets_for_ipv6_authorities() {
    for (sql, expected) in [
        ("SELECT port('http://2001:db8::80/')", 0),
        ("SELECT port('http://2001:db8::80/', 1234)", 1234),
        ("SELECT port('http://[2001:db8::1]:8123/path')", 8123),
        ("SELECT port('http://[2001:db8::1]/path', 2345)", 2345),
        ("SELECT port('http://example.com:8123/path')", 8123),
        ("SELECT port('example.com:80:90', 1234)", 1234),
        ("SELECT port('http://user:pass@example.com:443/')", 443),
        ("SELECT port('http://[::1]:0/')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
}
