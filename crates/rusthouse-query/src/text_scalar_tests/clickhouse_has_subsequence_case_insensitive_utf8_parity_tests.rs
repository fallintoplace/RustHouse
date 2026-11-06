use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn matches_case_insensitive_unicode_subsequences() {
    for (sql, expected) in [
        (
            "SELECT hasSubsequenceCaseInsensitiveUTF8('не прислоняться', 'не слонЯ')",
            1,
        ),
        ("SELECT hasSubsequenceCaseInsensitiveUTF8('Äpfel', 'äe')", 1),
        ("SELECT hasSubsequenceCaseInsensitiveUTF8('ДОБРО', 'до')", 1),
        ("SELECT hasSubsequenceCaseInsensitiveUTF8('AbCd', 'dc')", 0),
        ("SELECT hasSubsequenceCaseInsensitiveUTF8('AbCd', '')", 1),
        ("SELECT hasSubsequenceCaseInsensitiveUTF8('', 'a')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT hasSubsequenceCaseInsensitiveUTF8(NULL, 'a')"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT hasSubsequenceCaseInsensitiveUTF8('a', NULL)"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT hasSubsequenceCaseInsensitiveUTF8('a')").is_err());
}
