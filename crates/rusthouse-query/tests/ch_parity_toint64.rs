use rusthouse_query::{parse_select, Projection};
#[test]
fn clickhouse_toint64_constant_cast() {
    for (sql, expected) in [
        ("SELECT toInt64(42)", 42),
        ("SELECT toInt64(-99)", -99),
        ("SELECT toInt64(0)", 0),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for sql in ["SELECT toInt64()", "SELECT toInt64(1, 2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
