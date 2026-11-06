use super::*;
use crate::{ConstantValue, Projection, parse_select};

fn value(sql: &str) -> ConstantValue {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(items) if items.len() == 1 => items[0].value.clone(),
        Projection::Literal(number) => ConstantValue::Integer(number),
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn decodes_even_and_odd_hex_digit_counts() {
    assert_eq!(unhex_text("").unwrap(), "");
    assert_eq!(unhex_text("6869").unwrap(), "hi");
    assert_eq!(unhex_text("61A").unwrap(), "a\n");
    assert_eq!(unhex_text("4").unwrap(), "\u{0004}");
    assert_eq!(unhex_text("00").unwrap(), "\0");
    assert_eq!(unhex_text("C3A9").unwrap(), "é");
}

#[test]
fn rejects_malformed_or_non_utf8_text_constants() {
    assert!(unhex_text("G0").is_err());
    assert!(unhex_text("aé").is_err());
    assert!(unhex_text("FF").is_err());
    assert_eq!(
        value("SELECT unhex('6869')"),
        ConstantValue::Text("hi".into())
    );
    assert_eq!(value("SELECT unhex(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT unhex()").is_err());
    assert!(parse_select("SELECT unhex('61', '62')").is_err());
}
