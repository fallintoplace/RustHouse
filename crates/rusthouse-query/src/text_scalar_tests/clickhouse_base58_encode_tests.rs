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
fn encodes_bitcoin_alphabet_with_leading_zero_bytes() {
    assert_eq!(base58_encode("").unwrap(), "");
    assert_eq!(base58_encode("Hello").unwrap(), "9Ajdvzr");
    assert_eq!(base58_encode("Encoded").unwrap(), "3dc8KtHrwM");
    assert_eq!(base58_encode("\0\0A").unwrap(), "1128");
    assert_eq!(base58_encode("\0\0").unwrap(), "11");
}

#[test]
fn handles_nullable_sql_constant_and_input_bound() {
    assert_eq!(
        value("SELECT base58Encode('Hello')"),
        ConstantValue::Text("9Ajdvzr".into())
    );
    assert_eq!(value("SELECT base58Encode(NULL)"), ConstantValue::Null);
    assert!(parse_select("SELECT base58Encode()").is_err());
    assert!(parse_select("SELECT base58Encode('a', 'b')").is_err());
    assert!(base58_encode(&"a".repeat(4097)).is_err());
}
