use rusthouse_query::{Projection, parse_select};

#[test]
fn touint8_matches_bounded_clickhouse_integer_conversion() {
    for (sql, expected) in [
        ("SELECT toUInt8(255)", 255),
        ("SELECT toUInt8(256)", 0),
        ("SELECT toUInt8(-1)", 255),
        ("SELECT toUInt8(-257)", 255),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn touint8_rejects_bad_arguments() {
    for sql in [
        "SELECT toUInt8()",
        "SELECT toUInt8(1, 2)",
        "SELECT toUInt8('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
