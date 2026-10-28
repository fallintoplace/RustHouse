use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_minus_constants() {
    for (sql, expected) in [
        ("SELECT minus(9, 4)", 5),
        ("SELECT minus(-4, 5)", -9),
        ("SELECT minus(10, 2 * 3)", 4),
        ("SELECT minus(1, minus(4, 8))", 5),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn clickhouse_minus_rejects_invalid_inputs() {
    for sql in [
        "SELECT minus(1)",
        "SELECT minus(1, 2, 3)",
        "SELECT minus(9223372036854775807, -1)",
        "SELECT minus(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
