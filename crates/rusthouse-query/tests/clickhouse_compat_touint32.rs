use rusthouse_query::{Projection, parse_select};

#[test]
fn touint32_matches_bounded_clickhouse_integer_conversion() {
    for (sql, expected) in [
        ("SELECT toUInt32(4294967295)", 4294967295),
        ("SELECT toUInt32(4294967296)", 0),
        ("SELECT toUInt32(-1)", 4294967295),
        ("SELECT toUInt32(-4294967297)", 4294967295),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn touint32_rejects_bad_arguments() {
    for sql in [
        "SELECT toUInt32()",
        "SELECT toUInt32(1, 2)",
        "SELECT toUInt32('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
