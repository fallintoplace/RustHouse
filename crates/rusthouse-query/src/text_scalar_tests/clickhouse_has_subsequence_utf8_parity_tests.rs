use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn matches_ordered_unicode_codepoints() {
    for (sql, expected) in [
        (
            "SELECT hasSubsequenceUTF8('ReplicatedMergeTree', 'RepMT')",
            1,
        ),
        ("SELECT hasSubsequenceUTF8('a💫bç', 'aç')", 1),
        ("SELECT hasSubsequenceUTF8('a💫bç', 'ça')", 0),
        ("SELECT hasSubsequenceUTF8('Москва', 'Мса')", 1),
        ("SELECT hasSubsequenceUTF8('a💫b', '')", 1),
        ("SELECT hasSubsequenceUTF8('', 'a')", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT hasSubsequenceUTF8(NULL, 'a')"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT hasSubsequenceUTF8('a', NULL)"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT hasSubsequenceUTF8('a')").is_err());
}
