use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_bit_test_any_constants() {
    for (sql, expected) in [
        ("SELECT bitTestAny(43, 0, 2)", 1),
        ("SELECT bitTestAny(43, 4, 2)", 0),
        ("SELECT bitTestAny(-1, 63)", 1),
        ("SELECT bitTestAny(8, 1, 3)", 1),
        ("SELECT bitTestAny(0, 0)", 0),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn clickhouse_bit_test_any_rejects_invalid_inputs() {
    for sql in [
        "SELECT bitTestAny(1)",
        "SELECT bitTestAny(1, -1)",
        "SELECT bitTestAny(1, 64)",
        "SELECT bitTestAny(1, 0) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
