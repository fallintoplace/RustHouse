use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_if_int64_constants() {
    for (sql, expected) in [
        ("SELECT if(1, 7, 9)", 7),
        ("SELECT IF(0, 7, 9)", 9),
        ("SELECT if(-2, 2 + 3, 8)", 5),
        ("SELECT if(1, if(0, 3, 4), 2)", 4),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn clickhouse_if_int64_rejects_invalid_inputs() {
    for sql in [
        "SELECT if(1, 2)",
        "SELECT if(1, 2, 3, 4)",
        "SELECT if(1, 'yes', 0)",
        "SELECT if(1, 2, 3) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
