use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn hex_accepts_strings_and_int64_constants() {
    for (query, expected) in [
        ("SELECT hex(0)", ConstantValue::Text("00".into())),
        ("SELECT hex(10)", ConstantValue::Text("0A".into())),
        ("SELECT hex(255)", ConstantValue::Text("FF".into())),
        ("SELECT hex(256)", ConstantValue::Text("0100".into())),
        ("SELECT hex(65535)", ConstantValue::Text("FFFF".into())),
        ("SELECT hex('Click')", ConstantValue::Text("436C69636B".into())),
        ("SELECT hex_encode('Click')", ConstantValue::Text("436C69636B".into())),
        ("SELECT hex(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(query), expected, "{query}");
    }
    assert!(parse_select("SELECT hex(1, 2)").is_err());
    assert!(parse_select("SELECT hex_encode(255)").is_err());
}
