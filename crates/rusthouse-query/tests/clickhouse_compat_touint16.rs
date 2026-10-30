use rusthouse_query::{Projection, parse_select};

#[test]
fn touint16_matches_bounded_clickhouse_integer_conversion() {
    for (sql, expected) in [
        ("SELECT toUInt16(65535)", 65535),
        ("SELECT toUInt16(65536)", 0),
        ("SELECT toUInt16(-1)", 65535),
        ("SELECT toUInt16(-65537)", 65535),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn touint16_rejects_bad_arguments() {
    for sql in [
        "SELECT toUInt16()",
        "SELECT toUInt16(1, 2)",
        "SELECT toUInt16('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
