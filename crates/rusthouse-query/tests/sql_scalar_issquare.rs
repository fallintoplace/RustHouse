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
fn issquare_handles_signed_inputs_and_errors() {
    for (input, expected) in [
        ("-1", 0),
        ("0", 1),
        ("1", 1),
        ("9", 1),
        ("15", 0),
        ("9223372030926249001", 1),
        ("9223372036854775807", 0),
    ] {
        let sql = format!("SELECT isSquare({input})");
        assert_eq!(scalar(&sql), expected, "{sql}");
    }
    assert!(parse_select("SELECT isSquare()").is_err());
    assert!(parse_select("SELECT isSquare(1, 2)").is_err());

}
