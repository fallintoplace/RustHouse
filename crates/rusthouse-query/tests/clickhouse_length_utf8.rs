use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn counts_utf8_code_points_not_bytes() {
    for (sql, expected) in [
        ("SELECT lengthUTF8('')", 0),
        ("SELECT lengthUTF8('abc')", 3),
        ("SELECT lengthUTF8('München')", 7),
        ("SELECT lengthUTF8('😀é')", 2),
        ("SELECT lengthUTF8(reverse('é日'))", 2),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT lengthUTF8(NULL)"), ConstantValue::Null);
}

#[test]
fn checks_utf8_length_argument_count_and_type() {
    for sql in [
        "SELECT lengthUTF8()",
        "SELECT lengthUTF8(10)",
        "SELECT lengthUTF8('a', 'b')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
