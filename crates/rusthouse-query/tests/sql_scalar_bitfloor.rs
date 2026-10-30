use rusthouse_query::{parse_select, ConstantValue, Projection};

fn scalar(sql: &str) -> i64 {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(value) => value,
        Projection::Constants(values) => match values[0].value {
            ConstantValue::Integer(value) => value,
            ref other => panic!("unexpected value: {other:?}"),
        },
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn bitfloor_handles_signed_inputs_and_errors() {
    for (input, expected) in [
        ("0", 0),
        ("1", 1),
        ("2", 2),
        ("3", 2),
        ("9", 8),
        ("1024", 1024),
        ("9223372036854775807", 4611686018427387904),
    ] {
        let sql = format!("SELECT bitFloor({input})");
        assert_eq!(scalar(&sql), expected, "{sql}");
    }
    assert!(parse_select("SELECT bitFloor()").is_err());
    assert!(parse_select("SELECT bitFloor(1, 2)").is_err());
    assert!(parse_select("SELECT bitFloor(-1)").is_err());
}
