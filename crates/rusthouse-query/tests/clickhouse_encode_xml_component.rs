use rusthouse_query::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => columns[0].value.clone(),
        Projection::Literal(value) => ConstantValue::Integer(value),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn encode_xml_component_evaluates_constant_expressions() {
    for (sql, expected) in [
        ("SELECT encodeXMLComponent('<tag>')", ConstantValue::Text("&lt;tag&gt;".into())),
        ("SELECT encodeXMLComponent('a&b')", ConstantValue::Text("a&amp;b".into())),
        ("SELECT encodeXMLComponent('\"')", ConstantValue::Text("&quot;".into())),
        ("SELECT encodeXMLComponent(NULL)", ConstantValue::Null),
    ] {
        assert_eq!(value(sql), expected, "{sql}");
    }
}

#[test]
fn encode_xml_component_rejects_invalid_arguments() {
    for sql in [
        "SELECT encodeXMLComponent()",
        "SELECT encodeXMLComponent(1)",
        "SELECT encodeXMLComponent('x','y')",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
