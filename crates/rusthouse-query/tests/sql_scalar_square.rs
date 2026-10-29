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
fn square_handles_signed_inputs_and_errors() {
    for (input, expected) in [
        ("0", 0),
        ("3", 9),
        ("-4", 16),
        ("3037000499", 9223372030926249001),
    ] {
        let sql = format!("SELECT square({input})");
        assert_eq!(scalar(&sql), expected, "{sql}");
    }
    assert!(parse_select("SELECT square()").is_err());
    assert!(parse_select("SELECT square(1, 2)").is_err());
    assert!(parse_select("SELECT square(3037000500)").is_err());
}
