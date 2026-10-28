use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn trims_left_only_and_preserves_unicode() {
    assert_eq!(
        value("SELECT LTRIM('  éé foo  ')"),
        ConstantValue::Text("éé foo  ".into())
    );
    assert_eq!(
        value("SELECT ltrim('ééalpha', 'é')"),
        ConstantValue::Text("alpha".into())
    );
    assert_eq!(
        value("SELECT LTRIM('xyzAxy', 'xyz')"),
        ConstantValue::Text("Axy".into())
    );
    assert_eq!(
        value("SELECT LTRIM(' xyz ', '')"),
        ConstantValue::Text(" xyz ".into())
    );
    assert_eq!(value("SELECT LTRIM(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT LTRIM('x', NULL)"), ConstantValue::Null);
}

#[test]
fn ltrim_rejects_missing_and_extra_arguments() {
    for sql in [
        "SELECT LTRIM()",
        "SELECT LTRIM('a', 'b', 'c')",
        "SELECT LTRIM(2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
