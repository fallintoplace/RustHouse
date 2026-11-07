use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn counts_unicode_whitespace_separated_words() {
    for (sql, expected) in [
        ("SELECT countWords('hello  RustHouse')", ConstantValue::Integer(2)),
        ("SELECT countWords('foo bar baz')", ConstantValue::Integer(3)),
        ("SELECT countWords('')", ConstantValue::Integer(0)),
        ("SELECT countWords('   ')", ConstantValue::Integer(0)),
        ("SELECT countWords(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn validates_count_words_arity() {
    assert!(parse_select("SELECT countWords()").is_err());
    assert!(parse_select("SELECT countWords('a', 'b')").is_err());
}
