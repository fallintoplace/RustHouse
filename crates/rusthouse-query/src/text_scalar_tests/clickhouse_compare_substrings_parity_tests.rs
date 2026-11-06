use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn compares_byte_ranges_and_short_tails() {
    for (sql, expected) in [
        (
            "SELECT compareSubstrings('Saxony', 'Anglo-Saxon', 0, 6, 5)",
            0,
        ),
        ("SELECT compareSubstrings('abc', 'abd')", -1),
        ("SELECT compareSubstrings('abd', 'abc')", 1),
        ("SELECT compareSubstrings('abc', 'abc')", 0),
        ("SELECT compareSubstrings('abc', 'abc', 2, 2, 0)", 0),
        ("SELECT compareSubstrings('abc', 'abc', 99, 99, 7)", 0),
        ("SELECT compareSubstrings('ab', 'abc', 0, 0, 5)", -1),
        ("SELECT compareSubstrings('é', 'é', 1, 1, 1)", 0),
    ] {
        assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
    }
    assert_eq!(
        value("SELECT compareSubstrings(NULL, 'abc')"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT compareSubstrings('abc')").is_err());
    assert!(parse_select("SELECT compareSubstrings('abc', 'def', 1)").is_err());
    assert!(parse_select("SELECT compareSubstrings('abc', 'def', -1, 0, 2)").is_err());
}
