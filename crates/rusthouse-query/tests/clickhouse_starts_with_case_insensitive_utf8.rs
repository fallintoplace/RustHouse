use rusthouse_query::{ConstantValue, Projection, parse_select};
fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(v) => ConstantValue::Integer(v),
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn compares_case_insensitive_prefixes_and_suffixes() {
    assert_eq!(value("SELECT startsWithCaseInsensitiveUTF8('Привет','ПРИ')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT startsWithCaseInsensitiveUTF8('München','MÜN')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT startsWithCaseInsensitiveUTF8('Alpha','z')"), ConstantValue::Integer(0));
    assert_eq!(value("SELECT startsWithCaseInsensitiveUTF8('é','')"), ConstantValue::Integer(1));
    assert_eq!(value("SELECT startswithcaseinsensitiveutf8(NULL, 'x')"), ConstantValue::Null);
    assert_eq!(value("SELECT startswithcaseinsensitiveutf8('x', NULL)"), ConstantValue::Null);
}
#[test]
fn validates_argument_counts() {
    for sql in ["SELECT startswithcaseinsensitiveutf8()", "SELECT startswithcaseinsensitiveutf8('x')", "SELECT startswithcaseinsensitiveutf8(1,'x')", "SELECT startswithcaseinsensitiveutf8('x','y','z')"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
