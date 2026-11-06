use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn reverses_dot_slash_and_multibyte_separator_tokens() {
    for (sql, expected) in [
        (
            "SELECT reverseBySeparator('www.google.com')",
            "com.google.www",
        ),
        ("SELECT reverseBySeparator('a/b/c', '/')", "c/b/a"),
        ("SELECT reverseBySeparator('x::y::z', '::')", "z::y::x"),
        ("SELECT reverseBySeparator('.a.b.')", ".b.a."),
        ("SELECT reverseBySeparator('single')", "single"),
        ("SELECT reverseBySeparator('abcde', '')", "edcba"),
        ("SELECT reverseBySeparator('a💡b💡c', '💡')", "c💡b💡a"),
    ] {
        assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
    }
    assert_eq!(
        value("SELECT reverseBySeparator(NULL)"),
        ConstantValue::Null
    );
    assert_eq!(
        value("SELECT reverseBySeparator('x', NULL)"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT reverseBySeparator()").is_err());
    assert!(parse_select("SELECT reverseBySeparator('a', '.', 'b')").is_err());
}
