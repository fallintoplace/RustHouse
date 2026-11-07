use rusthouse_query::{parse_select, ConstantValue, Projection};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn jsonextractuint_examples() {
    for (sql, expected) in [
        ("SELECT JSONExtractUInt('{\"n\":42}', 'n')", ConstantValue::Integer(42)),
        ("SELECT JSONExtractUInt('{\"n\":-1}', 'n')", ConstantValue::Integer(0)),
        ("SELECT JSONExtractUInt('{\"n\":3.5}', 'n')", ConstantValue::Integer(0)),
        ("SELECT JSONExtractUInt('{}', 'missing')", ConstantValue::Integer(0)),
        ("SELECT JSONExtractUInt(NULL, 'n')", ConstantValue::Null),
        ("SELECT JSONExtractUInt('{\"n\":1}', NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn jsonextractuint_rejects_invalid_arguments() {
    for sql in [
        "SELECT JSONExtractUInt('{')",
        "SELECT JSONExtractUInt('{\"n\":9223372036854775808}', 'n')",
        "SELECT JSONEXTRACTUINT()",
        "SELECT JSONEXTRACTUINT(NULL, NULL, NULL)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
