use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql: &str) -> i64 {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(v) => v,
        Projection::Constants(items) if items.len()==1 => match items[0].value {
            ConstantValue::Integer(v) => v,
            ref other => panic!("expected integer, got {other:?}"),
        },
        other => panic!("unexpected projection: {other:?}"),
    }
}
#[test]
fn clickhouse_toint16_wraps_at_target_width() {
    assert_eq!(value("SELECT toInt16(32767)"), 32767);
    assert_eq!(value("SELECT toInt16(32768)"), -32768);
    assert_eq!(value("SELECT toInt16(-32769)"), 32767);
    assert_eq!(value("SELECT toInt16(65535)"), -1);
}
#[test]
fn clickhouse_toint16_rejects_invalid_arguments() {
    for sql in [
        "SELECT toInt16()",
        "SELECT toInt16(1, 2)",
        "SELECT toInt16('123')",
        "SELECT toInt16(1) FROM numbers(1)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
