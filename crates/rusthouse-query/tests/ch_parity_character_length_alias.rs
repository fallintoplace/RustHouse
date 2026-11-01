use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn character_length_counts_codepoints_not_bytes() {
    for (sql, expected) in [
        ("SELECT CHARACTER_LENGTH('é猫')", 2),
        ("SELECT cHaRaCtEr_LeNgTh('abc')", 3),
        ("SELECT CHAR_LENGTH('é猫')", 2),
        ("SELECT length('é猫')", 5),
        ("SELECT OCTET_LENGTH('é猫')", 5),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT CHARACTER_LENGTH(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT CHARACTER_LENGTH('x', 1)").is_err());
}
