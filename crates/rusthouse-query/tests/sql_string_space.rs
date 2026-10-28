use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn space_returns_requested_width() {
    assert_eq!(value("SELECT SPACE(0)"), ConstantValue::Text("".into()));
    assert_eq!(value("SELECT SPACE(3)"), ConstantValue::Text("   ".into()));
    assert_eq!(
        value("SELECT SPACE(2+2)"),
        ConstantValue::Text("    ".into())
    );
    assert_eq!(value("SELECT SPACE(NULL)"), ConstantValue::Null);
}

#[test]
fn space_checks_length_sign_and_arity() {
    for sql in [
        "SELECT SPACE(-1)",
        "SELECT SPACE(1048577)",
        "SELECT SPACE()",
        "SELECT SPACE(1, 2)",
        "SELECT SPACE('a')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
