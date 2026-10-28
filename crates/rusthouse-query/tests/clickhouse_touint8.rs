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
fn clickhouse_touint8_wraps_at_target_width() {
    assert_eq!(value("SELECT toUInt8(255)"), 255);
    assert_eq!(value("SELECT toUInt8(-1)"), 255);
    assert_eq!(value("SELECT toUInt8(256)"), 0);
    assert_eq!(value("SELECT toUInt8(257)"), 1);
}
#[test]
fn clickhouse_touint8_rejects_invalid_arguments() {
    for sql in [
        "SELECT toUInt8()",
        "SELECT toUInt8(1, 2)",
        "SELECT toUInt8('123')",
        "SELECT toUInt8(1) FROM numbers(1)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
