use rusthouse_query::{parse_select, Projection};
#[test]
fn clickhouse_touint32_constant_cast() {
    for (sql, expected) in [
        ("SELECT toUInt32(4294967295)", 4294967295),
        ("SELECT toUInt32(4294967296)", 0),
        ("SELECT toUInt32(-1)", 4294967295),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for sql in ["SELECT toUInt32()", "SELECT toUInt32(1, 2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
