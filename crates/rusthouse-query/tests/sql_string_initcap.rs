use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn initcap_uses_unicode_and_word_boundaries() {
    for (sql, expected) in [
        ("SELECT INITCAP('hELLO wORLD')", "Hello World"),
        ("SELECT initcap('éCOLE À PARIS')", "École À Paris"),
        ("SELECT INITCAP('a-B_c.d 42AB')", "A-B_C.D 42ab"),
        ("SELECT INITCAP('')", ""),
        ("SELECT INITCAP(LOWER('HEllo WORLD'))", "Hello World"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(value("SELECT INITCAP(NULL)"), ConstantValue::Null);
}

#[test]
fn initcap_rejects_bad_arguments() {
    for sql in ["SELECT INITCAP()", "SELECT INITCAP('a', 'b')", "SELECT INITCAP(3)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
