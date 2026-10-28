use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn trims_right_only_and_preserves_unicode() {
    assert_eq!(
        value("SELECT RTRIM('  foo  ')"),
        ConstantValue::Text("  foo".into())
    );
    assert_eq!(
        value("SELECT rtrim('alphaéé', 'é')"),
        ConstantValue::Text("alpha".into())
    );
    assert_eq!(
        value("SELECT RTRIM('xyAxyz', 'xyz')"),
        ConstantValue::Text("xyA".into())
    );
    assert_eq!(
        value("SELECT RTRIM(' xyz ', '')"),
        ConstantValue::Text(" xyz ".into())
    );
    assert_eq!(value("SELECT RTRIM(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT RTRIM('x', NULL)"), ConstantValue::Null);
}

#[test]
fn rtrim_rejects_missing_and_extra_arguments() {
    for sql in [
        "SELECT RTRIM()",
        "SELECT RTRIM('a', 'b', 'c')",
        "SELECT RTRIM(2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
