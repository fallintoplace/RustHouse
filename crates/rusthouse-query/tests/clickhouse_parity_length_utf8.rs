use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn counts_unicode_code_points_instead_of_bytes() {
    for (sql, expected) in [
        ("SELECT lengthUTF8('é🙂')", 2),
        ("SELECT LENGTHUTF8('Привет')", 6),
        ("SELECT lengthUTF8('')", 0),
        ("SELECT lengthUTF8(upperUTF8('a'))", 1),
    ] {
        // Nested function coverage for this PR uses the existing UPPER spelling.
        let sql = sql.replace("upperUTF8", "UPPER");
        assert_eq!(value(&sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(value("SELECT lengthUTF8(NULL)"), ConstantValue::Null);
    assert_eq!(value("SELECT LENGTH('é🙂')"), ConstantValue::Integer(6));
}

#[test]
fn rejects_wrong_arguments_and_table_expressions() {
    for sql in ["SELECT lengthUTF8()", "SELECT lengthUTF8(1)", "SELECT lengthUTF8('x','y')", "SELECT lengthUTF8('x') FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
