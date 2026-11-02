use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(integer) => ConstantValue::Integer(integer),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn crc32_ieee_uses_zero_initial_seed_without_final_inversion() {
    for (query, expected) in [
        ("SELECT CRC32IEEE('ClickHouse')", ConstantValue::Integer(3_089_448_422)),
        ("SELECT crc32ieee('123456789')", ConstantValue::Integer(771_566_984)),
        ("SELECT CRC32IEEE('')", ConstantValue::Integer(0)),
        ("SELECT CRC32IEEE(NULL)", ConstantValue::Null),
        ("SELECT CRC32('ClickHouse')", ConstantValue::Integer(1_538_217_360)),
    ] {
        assert_eq!(value(query), expected, "{query}");
    }
    assert!(parse_select("SELECT CRC32IEEE('a', 'b')").is_err());
}
