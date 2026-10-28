use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn coalesce_evaluates_constant_strings() {
    assert_eq!(
        value("SELECT COALESCE(NULL, 'backup')"),
        ConstantValue::Text("backup".into())
    );
    assert_eq!(
        value("SELECT coalesce('first', 'second')"),
        ConstantValue::Text("first".into())
    );
    assert_eq!(
        value("SELECT COALESCE(NULL, NULL, 'ok')"),
        ConstantValue::Text("ok".into())
    );
    assert_eq!(
        value("SELECT CONCAT(COALESCE(NULL, 'a'), 'b')"),
        ConstantValue::Text("ab".into())
    );
    assert_eq!(value("SELECT COALESCE(NULL, NULL)"), ConstantValue::Null);
}

#[test]
fn coalesce_rejects_bad_arguments_and_table_scans() {
    for sql in [
        "SELECT COALESCE()",
        "SELECT COALESCE('a',)",
        "SELECT COALESCE(1, 'a')",
        "SELECT COALESCE('x') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
