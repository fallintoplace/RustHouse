use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn jsonextractfloat_handles_constant_inputs() {
    for (sql, expected) in [
        ("SELECT JSONExtractFloat('{\"price\":1.25}', 'price')", ConstantValue::Decimal("1.25".into())),
        ("SELECT JSONExtractFloat('{\"price\":-0.5}', 'price')", ConstantValue::Decimal("-0.5".into())),
        ("SELECT JSONExtractFloat('{\"price\":\"1.5\"}', 'price')", ConstantValue::Decimal("0".into())),
        ("SELECT JSONExtractFloat('{\"price\":1.25}', 'missing')", ConstantValue::Decimal("0".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT jsonextractfloat(NULL, NULL)"), ConstantValue::Null);
}

#[test]
fn jsonextractfloat_rejects_invalid_arguments() {
    for sql in [
        "SELECT JSONExtractFloat('{')",
        "SELECT JSONExtractFloat('{}', 'a', 'b')",
        "SELECT JSONExtractFloat(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
