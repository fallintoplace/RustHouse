use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn damerau_levenshtein_counts_transpositions() {
    for (query, expected) in [
        ("SELECT damerauLevenshteinDistance('ca', 'ac')", ConstantValue::Integer(1)),
        ("SELECT damerauLevenshteinDistance('CA', 'ABC')", ConstantValue::Integer(2)),
        ("SELECT damerauLevenshteinDistance('kitten', 'sitting')", ConstantValue::Integer(3)),
        ("SELECT damerauLevenshteinDistance('', 'abc')", ConstantValue::Integer(3)),
        ("SELECT damerauLevenshteinDistance('same', 'same')", ConstantValue::Integer(0)),
        ("SELECT damerauLevenshteinDistance(NULL, 'abc')", ConstantValue::Null),
    ] {
        assert_eq!(value(query), expected, "{query}");
    }
    assert!(parse_select("SELECT damerauLevenshteinDistance('a')").is_err());
    assert!(parse_select("SELECT damerauLevenshteinDistance(repeat('a', 1200), repeat('b', 1200))").is_err());
}
