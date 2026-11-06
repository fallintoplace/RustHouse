use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn formats_signed_integers_at_requested_scale() {
    for (sql, expected) in [
        ("SELECT toDecimalString(5, 3)", "5.000"),
        ("SELECT toDecimalString(-7, 2)", "-7.00"),
        ("SELECT toDecimalString(0, 0)", "0"),
        ("SELECT toDecimalString(-1, 0)", "-1"),
        ("SELECT toDecimalString(12, 1)", "12.0"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    let high_precision = match value("SELECT toDecimalString(3, 77)") {
        ConstantValue::Text(value) => value,
        other => panic!("unexpected value: {other:?}"),
    };
    assert_eq!(high_precision, format!("3.{}", "0".repeat(77)));
    assert_eq!(
        value("SELECT toDecimalString(NULL, 2)"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT toDecimalString(4, NULL)"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT toDecimalString(4)").is_err());
    assert!(parse_select("SELECT toDecimalString(4, -1)").is_err());
    assert!(parse_select("SELECT toDecimalString(4, 78)").is_err());
}
