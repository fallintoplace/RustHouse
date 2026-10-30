use rusthouse_query::{Projection, parse_select};

#[test]
fn toint32_matches_bounded_clickhouse_integer_conversion() {
    for (sql, expected) in [
        ("SELECT toInt32(2147483647)", 2147483647),
        ("SELECT toInt32(2147483648)", -2147483648),
        ("SELECT toInt32(-2147483649)", 2147483647),
        ("SELECT toInt32(4294967296)", 0),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn toint32_rejects_bad_arguments() {
    for sql in [
        "SELECT toInt32()",
        "SELECT toInt32(1, 2)",
        "SELECT toInt32('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
