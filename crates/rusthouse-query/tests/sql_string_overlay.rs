use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn overlay_uses_character_offsets_and_default_width() {
    assert_eq!(value("SELECT OVERLAY('abcdef', 'xy', 3)"), ConstantValue::Text("abxyef".into()));
    assert_eq!(value("SELECT OVERLAY('abcdef', 'Z', 3, 2)"), ConstantValue::Text("abZef".into()));
    assert_eq!(value("SELECT OVERLAY('é🙂x', 'Z', 2, 1)"), ConstantValue::Text("éZx".into()));
    assert_eq!(value("SELECT OVERLAY('abc', 'Z', 99)"), ConstantValue::Text("abcZ".into()));
    assert_eq!(value("SELECT OVERLAY('abc', '', 2, 1)"), ConstantValue::Text("ac".into()));
    assert_eq!(value("SELECT OVERLAY('a', NULL, 1)"), ConstantValue::Null);
    assert_eq!(value("SELECT OVERLAY('a', 'b', 1, NULL)"), ConstantValue::Null);
}

#[test]
fn overlay_rejects_negative_length_and_position() {
    for sql in ["SELECT OVERLAY('abc', 'x', 0)", "SELECT OVERLAY('abc', 'x', -1)", "SELECT OVERLAY('abc', 'x', 1, -1)", "SELECT OVERLAY('abc')", "SELECT OVERLAY('a', 'b', 1, 1, 1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
