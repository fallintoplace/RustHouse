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
fn decodes_bitcoin_base58_and_preserves_leading_zero_bytes() {
    assert_eq!(base58_decode("").unwrap(), "");
    assert_eq!(base58_decode("9Ajdvzr").unwrap(), "Hello");
    assert_eq!(base58_decode("3dc8KtHrwM").unwrap(), "Encoded");
    assert_eq!(base58_decode("1128").unwrap(), "\0\0A");
    assert_eq!(base58_decode("11").unwrap(), "\0\0");
    assert_eq!(base58_decode("2").unwrap(), "\u{0001}");
}

#[test]
fn validates_alphabet_utf8_and_argument_count() {
    for invalid in ["0", "O", "I", "l", "invalid"] {
        assert!(base58_decode(invalid).is_err(), "{invalid}");
    }
    assert!(base58_decode("5Q").is_err());
    assert!(base58_decode(&"z".repeat(4097)).is_err());
    assert_eq!(
        value("SELECT base58Decode('9Ajdvzr')"),
        ConstantValue::Text("Hello".into())
    );
    assert_eq!(value("SELECT base58Decode(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT base58Decode()").is_err());
    assert!(parse_select("SELECT base58Decode('1', '2')").is_err());
}
