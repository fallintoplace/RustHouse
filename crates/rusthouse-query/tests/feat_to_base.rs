use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn to_base_handles_constant_inputs() {
    for (sql, expected) in [
        ("SELECT to_base(255,16)", ConstantValue::Text("FF".into())),
        ("SELECT to_base(-15,2)", ConstantValue::Text("-1111".into())),
        ("SELECT to_base(0,36)", ConstantValue::Text("0".into())),
        ("SELECT to_base(35,36)", ConstantValue::Text("Z".into())),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
    assert_eq!(value("SELECT to_base(NULL, NULL)"), ConstantValue::Null);
}

#[test]
fn to_base_rejects_invalid_arguments() {
    for sql in [
        "SELECT to_base(42,1)",
        "SELECT to_base(42,37)",
        "SELECT to_base(42)",
        "SELECT to_base('x',16)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
