use rusthouse_query::{Projection, parse_select};

#[test]
fn clickhouse_bit_hamming_distance_counts_mismatched_bits() {
    for (sql, expected) in [
        ("SELECT bitHammingDistance(5, 3)", 2),
        ("SELECT bitHammingDistance(-1, 0)", 64),
        ("SELECT bitHammingDistance(3, 3)", 0),
        ("SELECT bitHammingDistance(1, bitNot(0))", 63),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for invalid in ["SELECT bitHammingDistance(1)", "SELECT bitHammingDistance(1, 2, 3)"] {
        assert!(parse_select(invalid).is_err(), "{invalid}");
    }
}
