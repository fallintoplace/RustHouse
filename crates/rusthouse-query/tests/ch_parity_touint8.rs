use rusthouse_query::{parse_select, Projection};
#[test]
fn clickhouse_touint8_constant_cast() {
    for (sql, expected) in [
        ("SELECT toUInt8(255)", 255),
        ("SELECT toUInt8(256)", 0),
        ("SELECT toUInt8(-1)", 255),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for sql in ["SELECT toUInt8()", "SELECT toUInt8(1, 2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
