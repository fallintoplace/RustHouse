use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn matches_unicode_suffixes_and_empty_needle() {
    for (sql, expected) in [
        ("SELECT endsWithCaseInsensitiveUTF8('данных', 'ЫХ')", 1),
        ("SELECT endsWithCaseInsensitiveUTF8('Äpfel', 'FEL')", 1),
        (
            "SELECT endsWithCaseInsensitiveUTF8('ClickHouse', 'HOUSE')",
            1,
        ),
        (
            "SELECT endsWithCaseInsensitiveUTF8('ClickHouse', 'click')",
            0,
        ),
        ("SELECT endsWithCaseInsensitiveUTF8('Ёжик', '')", 1),
        ("SELECT endsWithCaseInsensitiveUTF8('', '')", 1),
        ("SELECT endsWithCaseInsensitiveUTF8('', 'a')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT endsWithCaseInsensitiveUTF8(NULL, 'a')"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT endsWithCaseInsensitiveUTF8('a')").is_err());
    assert!(parse_select("SELECT endsWithCaseInsensitiveUTF8('a', 'b', 'c')").is_err());
}
