use super::{Test, compile_in};
use crate::Operator;

#[test]
fn one_uuid_member_uses_binary_comparison() {
    let value = "61f0c404-5cb3-11e7-907b-a6006ad3dba0".to_owned();
    for negated in [false, true] {
        let compiled = compile_in("UUID", std::slice::from_ref(&value), negated).unwrap();
        match compiled {
            Test::UuidCompare(operator, bytes) => {
                assert!(matches!(
                    (operator, negated),
                    (Operator::Eq, false) | (Operator::Ne, true)
                ));
                assert_eq!(bytes[0], 0x61);
                assert_eq!(bytes[15], 0xa0);
            }
            _ => panic!("single UUID IN should use fixed-width comparison"),
        }
    }
}
