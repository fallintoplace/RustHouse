use rusthouse_query::{parse_select, Projection};
#[test]
fn clickhouse_toint32_constant_cast() {
    for (sql, expected) in [
        ("SELECT toInt32(2147483647)", 2147483647),
        ("SELECT toInt32(2147483648)", -2147483648),
        ("SELECT toInt32(4294967295)", -1),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for sql in ["SELECT toInt32()", "SELECT toInt32(1, 2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
