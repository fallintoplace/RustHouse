use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn empty_matches_clickhouse_string_semantics() {
    for (sql, expected) in [
        ("SELECT empty('')", 1),
        ("SELECT EMPTY('x')", 0),
        ("SELECT empty('🙂')", 0),
        ("SELECT empty(CONCAT('', ''))", 1),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT empty(NULL)"), ConstantValue::Null);
}

#[test]
fn empty_rejects_non_string_and_table_inputs() {
    for sql in ["SELECT empty()", "SELECT empty(0)", "SELECT empty('a','b')", "SELECT empty('a') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
