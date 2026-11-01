use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn translates_unicode_and_deletes_unmapped_characters() {
    assert_eq!(
        value("SELECT TRANSLATE('abracadabra', 'abc', 'XYZ')"),
        ConstantValue::Text("XYrXZXdXYrX".into())
    );
    assert_eq!(
        value("SELECT TRANSLATEUTF8('aé🙂b', 'é🙂', 'E')"),
        ConstantValue::Text("aEb".into())
    );
    assert_eq!(
        value("SELECT TRANSLATE('abc', 'aa', 'XY')"),
        ConstantValue::Text("Ybc".into())
    );
    assert_eq!(
        value("SELECT TRANSLATE('abc', '', '')"),
        ConstantValue::Text("abc".into())
    );
    assert_eq!(
        value("SELECT TRANSLATE('abc', 'bc', 'X')"),
        ConstantValue::Text("aX".into())
    );
    assert_eq!(
        value("SELECT TRANSLATE('aé🙂b', 'ab', 'XY')"),
        ConstantValue::Text("Xé🙂Y".into())
    );
    assert_eq!(
        value("SELECT TRANSLATE(NULL, 'a', 'b')"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT TRANSLATE('a', NULL, 'b')"),
        ConstantValue::Null
    );
}

#[test]
fn translate_rejects_wrong_arity() {
    for sql in [
        "SELECT TRANSLATE()",
        "SELECT TRANSLATE('a', 'b')",
        "SELECT TRANSLATE('a', 'b', 'c', 'd')",
        "SELECT TRANSLATE('abc', 'a', 'bc')",
        "SELECT TRANSLATE('aé', 'é', 'E')",
        "SELECT TRANSLATEUTF8('abc', 'a', 'bc')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
