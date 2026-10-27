use rusthouse_query::{ConstantValue, Projection, parse_select};

#[test]
fn supports_popcount_not_and_bittest_on_signed_64_bit_patterns() {
    for (sql, expected) in [
        ("SELECT bitCount(0)", 0),
        ("SELECT BITCOUNT(15)", 4),
        ("SELECT bitCount(-1)", 64),
        ("SELECT bitCount(-9223372036854775808)", 1),
        ("SELECT bitCount(9223372036854775807)", 63),
        ("SELECT bitNot(0)", -1),
        ("SELECT BITNOT(-1)", 0),
        ("SELECT BITNOT(-9223372036854775808)", i64::MAX),
        ("SELECT bitTest(5, 0)", 1),
        ("SELECT bitTest(5, 1)", 0),
        ("SELECT bitTest(5, 2)", 1),
        ("SELECT BITTEST(-1, 63)", 1),
        ("SELECT bitTest(1, 63)", 0),
        ("SELECT bitTest(bitNot(0), 63)", 1),
        ("SELECT bitCount(bitAnd(15, 10))", 2),
        ("SELECT bitNot(bitNot(123)) + 1", 124),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }

    let query = parse_select("SELECT bitCount(7) AS bits, bitTest(7, 2) AS set").unwrap();
    let Projection::Constants(columns) = query.projection else {
        panic!("expected two scalar columns");
    };
    assert_eq!(columns[0].value, ConstantValue::Integer(3));
    assert_eq!(columns[1].value, ConstantValue::Integer(1));
}

#[test]
fn bit_introspection_rejects_invalid_arity_indices_and_sources() {
    for sql in [
        "SELECT bitCount()",
        "SELECT bitCount(1, 2)",
        "SELECT bitNot()",
        "SELECT bitNot(1, 2)",
        "SELECT bitTest(1)",
        "SELECT bitTest(1, -1)",
        "SELECT bitTest(1, 64)",
        "SELECT bitTest(1, 1, 2)",
        "SELECT bitTest(1,)",
        "SELECT bitTest(1, 'x')",
        "SELECT bitTest(1, 1",
        "SELECT bitCount(1) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
