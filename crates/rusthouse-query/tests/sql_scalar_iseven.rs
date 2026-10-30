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
fn iseven_handles_signed_inputs_and_errors() {
    for (input, expected) in [
        ("0", 1),
        ("1", 0),
        ("2", 1),
        ("-2", 1),
        ("-3", 0),
    ] {
        let sql = format!("SELECT isEven({input})");
        assert_eq!(scalar(&sql), expected, "{sql}");
    }
    assert!(parse_select("SELECT isEven()").is_err());
    assert!(parse_select("SELECT isEven(1, 2)").is_err());

}
