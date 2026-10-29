use rusthouse_query::{parse_select, Projection};
#[test]
fn clickhouse_touint16_constant_cast() {
    for (sql, expected) in [
        ("SELECT toUInt16(65535)", 65535),
        ("SELECT toUInt16(65536)", 0),
        ("SELECT toUInt16(-1)", 65535),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for sql in ["SELECT toUInt16()", "SELECT toUInt16(1, 2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
