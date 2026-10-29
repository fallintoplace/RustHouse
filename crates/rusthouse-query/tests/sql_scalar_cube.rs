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
fn cube_handles_signed_inputs_and_errors() {
    for (input, expected) in [
        ("0", 0),
        ("3", 27),
        ("-4", -64),
        ("-2097152", -9223372036854775808),
        ("2097151", 9223358842721533951),
    ] {
        let sql = format!("SELECT cube({input})");
        assert_eq!(scalar(&sql), expected, "{sql}");
    }
    assert!(parse_select("SELECT cube()").is_err());
    assert!(parse_select("SELECT cube(1, 2)").is_err());
    assert!(parse_select("SELECT cube(2097152)").is_err());
}
