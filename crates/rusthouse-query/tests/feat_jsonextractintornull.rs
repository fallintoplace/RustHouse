use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn integer_json_values_are_extracted() {
    for (sql, expected) in [
        ("SELECT JSONExtractIntOrNull('{\"n\":42}', 'n')", ConstantValue::Integer(42)),
        ("SELECT JSONExtractIntOrNull('{\"n\":0}', 'n')", ConstantValue::Integer(0)),
        ("SELECT JSONExtractIntOrNull('{\"n\":-6}', 'n')", ConstantValue::Integer(-6)),
        ("SELECT JSONExtractIntOrNull('17')", ConstantValue::Integer(17)),
        ("SELECT JSONExtractIntOrNull('{\"n\":1.25}', 'n')", ConstantValue::Null),
        ("SELECT JSONExtractIntOrNull('{\"n\":\"17\"}', 'n')", ConstantValue::Null),
        ("SELECT JSONExtractIntOrNull('{\"n\":false}', 'n')", ConstantValue::Null),
        ("SELECT JSONExtractIntOrNull('{\"n\":null}', 'n')", ConstantValue::Null),
        ("SELECT JSONExtractIntOrNull('{\"n\":5}', 'missing')", ConstantValue::Null),
        ("SELECT JSONExtractIntOrNull('{\"n\":9223372036854775808}', 'n')", ConstantValue::Null),
        ("SELECT JSONExtractIntOrNull(NULL, 'n')", ConstantValue::Null),
        ("SELECT JSONExtractIntOrNull('{\"n\":0}', NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn bad_json_and_arguments_are_rejected() {
    for sql in [
        "SELECT JSONExtractIntOrNull('{')",
        "SELECT JSONExtractIntOrNull()",
        "SELECT JSONExtractIntOrNull(1)",
        "SELECT JSONExtractIntOrNull('{}', 'a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
