use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_bitwise_and_or_xor_with_signed_integer_constants() {
    for (query, expected) in [
        ("SELECT bitAnd(7, 3)", 3),
        ("SELECT BITOR(4, 2)", 6),
        ("SELECT bitXor(7, 3)", 4),
        ("SELECT bitAnd(-1, 3)", 3),
        ("SELECT bitXor(-1, 3)", -4),
        ("SELECT bitOr(bitAnd(7, 3), 8)", 11),
        ("SELECT 2 * bitAnd(7, 3) + 1", 7),
        ("SELECT bitAnd(2 + 4, 3 * 2)", 6),
    ] {
        assert_eq!(
            parse_select(query).unwrap().projection,
            Projection::Literal(expected),
            "{query}"
        );
    }
}

#[test]
fn rejects_invalid_bitwise_arity_and_nesting() {
    for query in [
        "SELECT bitAnd(1)",
        "SELECT bitOr(1, 2, 3)",
        "SELECT bitXor(1,)",
        "SELECT bitAnd()",
        "SELECT bitAnd(1, 2",
        "SELECT bitOr(1, 'text')",
        "SELECT bitAnd(1, 2) FROM numbers(3)",
    ] {
        assert!(parse_select(query).is_err(), "{query}");
    }
    let deeply_nested = format!("SELECT {}1{}", "bitAnd(1,".repeat(20), ")".repeat(20));
    assert!(parse_select(&deeply_nested).is_err());
}
