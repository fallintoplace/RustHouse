use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn matches_unicode_prefixes_and_empty_needle() {
    for (sql, expected) in [
        (
            "SELECT startsWithCaseInsensitiveUTF8('приставка', 'ПРИ')",
            1,
        ),
        ("SELECT startsWithCaseInsensitiveUTF8('Äpfel', 'äP')", 1),
        (
            "SELECT startsWithCaseInsensitiveUTF8('ClickHouse', 'cli')",
            1,
        ),
        (
            "SELECT startsWithCaseInsensitiveUTF8('ClickHouse', 'house')",
            0,
        ),
        ("SELECT startsWithCaseInsensitiveUTF8('Ёжик', '')", 1),
        ("SELECT startsWithCaseInsensitiveUTF8('', '')", 1),
        ("SELECT startsWithCaseInsensitiveUTF8('', 'a')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT startsWithCaseInsensitiveUTF8(NULL, 'a')"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT startsWithCaseInsensitiveUTF8('a')").is_err());
    assert!(parse_select("SELECT startsWithCaseInsensitiveUTF8('a', 'b', 'c')").is_err());
}
