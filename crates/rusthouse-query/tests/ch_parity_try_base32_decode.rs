use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn tries_base32_decode_with_empty_on_error() {
    for (sql, expected) in [
        ("SELECT tryBase32Decode('IVXGG33EMVSA====')", "Encoded"),
        ("SELECT tryBase32Decode('')", ""),
        ("SELECT tryBase32Decode('bad')", ""),
        ("SELECT tryBase32Decode('IVXGG33EMVSA===!')", ""),
        ("SELECT tryBase32Decode('MZXW6===')", "foo"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT tryBase32Decode(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT base32Decode('bad')").is_err());
    assert!(parse_select("SELECT tryBase32Decode('x', 'y')").is_err());
}
