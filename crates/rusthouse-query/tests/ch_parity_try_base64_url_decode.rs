use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn tries_url_safe_base64_decode_with_empty_on_error() {
    for (sql, expected) in [
        (
            "SELECT tryBase64URLDecode('aHR0cHM6Ly9jbGlja2hvdXNlLmNvbQ')",
            "https://clickhouse.com",
        ),
        ("SELECT tryBase64URLDecode('SGVsbG8')", "Hello"),
        ("SELECT tryBase64URLDecode('')", ""),
        ("SELECT tryBase64URLDecode('?')", ""),
        ("SELECT tryBase64URLDecode('!')", ""),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT tryBase64URLDecode(NULL)"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT base64URLDecode('?')").is_err());
    assert!(parse_select("SELECT tryBase64URLDecode('x', 'y')").is_err());
}
