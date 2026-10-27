use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn chr_creates_valid_unicode_scalars() {
    assert_eq!(value("SELECT CHR(65)"), ConstantValue::Text("A".into()));
    assert_eq!(value("SELECT chr(233)"), ConstantValue::Text("é".into()));
    assert_eq!(value("SELECT CHR(128578)"), ConstantValue::Text("🙂".into()));
    assert_eq!(value("SELECT CHR(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT CHR(ASCII('B'))"), ConstantValue::Text("B".into()));
}

#[test]
fn chr_rejects_invalid_scalars_and_arity() {
    for sql in ["SELECT CHR(0)", "SELECT CHR(-1)", "SELECT CHR(55296)", "SELECT CHR(1114112)", "SELECT CHR()", "SELECT CHR(1, 2)", "SELECT CHR('A')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
