use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn substring_index_handles_direction_and_unicode() {
    assert_eq!(
        value("SELECT SUBSTRING_INDEX('a,b,c', ',', 2)"),
        ConstantValue::Text("a,b".into())
    );
    assert_eq!(
        value("SELECT SUBSTRING_INDEX('a,b,c', ',', -2)"),
        ConstantValue::Text("b,c".into())
    );
    assert_eq!(
        value("SELECT SUBSTRING_INDEX('a,b,c', ',', 0)"),
        ConstantValue::Text("".into())
    );
    assert_eq!(
        value("SELECT SUBSTRING_INDEX('a,b,c', ',', 99)"),
        ConstantValue::Text("a,b,c".into())
    );
    assert_eq!(
        value("SELECT SUBSTRING_INDEX('é🙂é', '🙂', -1)"),
        ConstantValue::Text("é".into())
    );
    assert_eq!(
        value("SELECT SUBSTRING_INDEX('ab', '', 1)"),
        ConstantValue::Text("".into())
    );
    assert_eq!(
        value("SELECT SUBSTRING_INDEX('a', 'x', -9223372036854775807)"),
        ConstantValue::Text("a".into())
    );
    assert_eq!(
        value("SELECT SUBSTRING_INDEX(NULL, ',', 1)"),
        ConstantValue::Null
    );
}

#[test]
fn substring_index_rejects_wrong_arity() {
    for sql in [
        "SELECT SUBSTRING_INDEX('a')",
        "SELECT SUBSTRING_INDEX('a', ',')",
        "SELECT SUBSTRING_INDEX('a', ',', 1, 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
