use rusthouse_query::{Projection, parse_select};

#[test]
fn toint16_preserves_clickhouse_integer_narrowing() {
    for (sql, expected) in [
        ("SELECT toInt16(32767)", 32767),
        ("SELECT toInt16(32768)", -32768),
        ("SELECT toInt16(65535)", -1),
        ("SELECT toInt16(-32769)", 32767),
        ("SELECT 2 + toInt16(65536)", 2),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn toint16_rejects_bad_arity_and_non_integer_inputs() {
    for sql in [
        "SELECT toInt16()",
        "SELECT toInt16(1, 2)",
        "SELECT toInt16('bad')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
