use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn encodes_reserved_bytes_and_preserves_nulls() {
    assert_eq!(
        value("SELECT encodeURLComponent('a b/c?d=e')"),
        ConstantValue::Text("a%20b%2Fc%3Fd%3De".into()),
    );
    assert_eq!(
        value("SELECT encodeURLComponent('café')"),
        ConstantValue::Text("caf%C3%A9".into()),
    );
    assert_eq!(
        value("SELECT encodeURLComponent('')"),
        ConstantValue::Text(String::new()),
    );
    assert_eq!(
        value("SELECT encodeURLComponent(NULL)"),
        ConstantValue::Null
    );
    assert!(parse_select("SELECT encodeURLComponent()").is_err());
    assert!(parse_select("SELECT encodeURLComponent('a', 'b')").is_err());
}
