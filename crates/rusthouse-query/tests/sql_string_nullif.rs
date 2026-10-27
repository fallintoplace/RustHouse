use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn nullif_evaluates_constant_strings() {
    assert_eq!(value("SELECT NULLIF('x', 'y')"), ConstantValue::Text("x".into()));
    assert_eq!(value("SELECT NULLIF('x', NULL)"), ConstantValue::Text("x".into()));
    assert_eq!(value("SELECT CONCAT_WS(',', NULLIF('x','x'), 'z')"), ConstantValue::Text("z".into()));
    assert_eq!(value("SELECT NULLIF('x','x')"), ConstantValue::Null);
    assert_eq!(value("SELECT NULLIF(NULL, 'x')"), ConstantValue::Null);
    assert_eq!(value("SELECT NULLIF('', '')"), ConstantValue::Null);
}

#[test]
fn nullif_rejects_bad_arguments_and_table_scans() {
    for sql in [
        "SELECT NULLIF()",
        "SELECT NULLIF('a')",
        "SELECT NULLIF('a','b','c')",
        "SELECT NULLIF('a',2)",
        "SELECT NULLIF('x','y') FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
