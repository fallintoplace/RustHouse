use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql:&str)->ConstantValue{
 match parse_select(sql).unwrap().projection{
 Projection::Constants(v) if v.len()==1=>v[0].value.clone(),
 Projection::Literal(n)=>ConstantValue::Integer(n),
 other=>panic!("unexpected projection {other:?}"),
 }
}
#[test]fn converts_valid_and_invalid_strings(){
 assert_eq!(value("SELECT toUInt8OrZero('255')"),ConstantValue::Integer(255));
 assert_eq!(value("SELECT toUInt8OrZero('256')"),ConstantValue::Integer(0));
 assert_eq!(value("SELECT toUInt8OrZero('-1')"),ConstantValue::Integer(0));
 assert_eq!(value("SELECT toUInt8OrZero('bad')"),ConstantValue::Integer(0));
 assert_eq!(value("SELECT toUInt8OrZero('4294967295')"),ConstantValue::Integer(255));
 assert_eq!(value("SELECT toUInt8OrZero(NULL)"),ConstantValue::Null);
}
#[test]fn rejects_invalid_argument_types_and_arity(){
 for sql in ["SELECT toUInt8OrZero()","SELECT toUInt8OrZero(123)","SELECT toUInt8OrZero('1','2')","SELECT toUInt8OrZero('1') FROM numbers(1)"]{assert!(parse_select(sql).is_err(),"{sql}");}
}