use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_bit_test_all_constants() {
    for (sql, expected) in [
        ("SELECT bitTestAll(43, 0, 1, 3, 5)", 1),
        ("SELECT bitTestAll(43, 0, 1, 3, 5, 2)", 0),
        ("SELECT bitTestAll(-1, 63, 0)", 1),
        ("SELECT bitTestAll(0, 0)", 0),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn clickhouse_bit_test_all_rejects_invalid_inputs() {
    for sql in [
        "SELECT bitTestAll(1)",
        "SELECT bitTestAll(1, -1)",
        "SELECT bitTestAll(1, 64)",
        "SELECT bitTestAll(1, 0) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
