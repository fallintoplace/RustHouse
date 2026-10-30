use rusthouse_query::{Projection, parse_select};

#[test]
fn toint64_matches_bounded_clickhouse_integer_conversion() {
    for (sql, expected) in [
        ("SELECT toInt64(0)", 0),
        ("SELECT toInt64(-42)", -42),
        ("SELECT toInt64(2147483648)", 2147483648),
        ("SELECT toInt64(9000000000000000000)", 9000000000000000000),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn toint64_rejects_bad_arguments() {
    for sql in [
        "SELECT toInt64()",
        "SELECT toInt64(1, 2)",
        "SELECT toInt64('invalid')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
