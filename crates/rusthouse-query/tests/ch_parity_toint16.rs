use rusthouse_query::{parse_select, Projection};
#[test]
fn clickhouse_int16_cast_wraps_integer_input() {
    for (sql, expected) in [
        ("SELECT toInt16(32767)", 32767),
        ("SELECT toInt16(32768)", -32768),
        ("SELECT toInt16(65535)", -1),
        ("SELECT toInt16(-32769)", 32767),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for sql in ["SELECT toInt16()", "SELECT toInt16(1, 2)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
