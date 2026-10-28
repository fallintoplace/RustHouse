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
fn clickhouse_toint8_wraps_at_target_width() {
    assert_eq!(value("SELECT toInt8(127)"), 127);
    assert_eq!(value("SELECT toInt8(128)"), -128);
    assert_eq!(value("SELECT toInt8(-129)"), 127);
    assert_eq!(value("SELECT toInt8(255)"), -1);
    assert_eq!(value("SELECT toInt8(toInt8(258))"), 2);
}
#[test]
fn clickhouse_toint8_rejects_invalid_arguments() {
    for sql in [
        "SELECT toInt8()",
        "SELECT toInt8(1, 2)",
        "SELECT toInt8('123')",
        "SELECT toInt8(1) FROM numbers(1)",
    ] { assert!(parse_select(sql).is_err(), "{sql}"); }
}
