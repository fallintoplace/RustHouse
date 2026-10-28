use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_bit_hamming_distance_constants() {
    for (sql, expected) in [
        ("SELECT bitHammingDistance(0, 0)", 0),
        ("SELECT bitHammingDistance(0, 15)", 4),
        ("SELECT bitHammingDistance(5, 3)", 2),
        ("SELECT bitHammingDistance(-1, 0)", 64),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn clickhouse_bit_hamming_distance_rejects_invalid_inputs() {
    for sql in [
        "SELECT bitHammingDistance(1)",
        "SELECT bitHammingDistance(1, 2, 3)",
        "SELECT bitHammingDistance(1, 2) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
