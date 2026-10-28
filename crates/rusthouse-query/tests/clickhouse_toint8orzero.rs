use rusthouse_query::{parse_select, ConstantValue, Projection};
fn value(sql:&str)->ConstantValue{
 match parse_select(sql).unwrap().projection{
 Projection::Constants(v) if v.len()==1=>v[0].value.clone(),
 Projection::Literal(n)=>ConstantValue::Integer(n),
 other=>panic!("unexpected projection {other:?}"),
 }
}
#[test]fn converts_valid_and_invalid_strings(){
 assert_eq!(value("SELECT toInt8OrZero('127')"),ConstantValue::Integer(127));
 assert_eq!(value("SELECT toInt8OrZero('128')"),ConstantValue::Integer(-128));
 assert_eq!(value("SELECT toInt8OrZero('-129')"),ConstantValue::Integer(127));
 assert_eq!(value("SELECT toInt8OrZero('bad')"),ConstantValue::Integer(0));
 assert_eq!(value("SELECT toInt8OrZero('1.5')"),ConstantValue::Integer(0));
 assert_eq!(value("SELECT toInt8OrZero(NULL)"),ConstantValue::Null);
}
#[test]fn rejects_invalid_argument_types_and_arity(){
 for sql in ["SELECT toInt8OrZero()","SELECT toInt8OrZero(123)","SELECT toInt8OrZero('1','2')","SELECT toInt8OrZero('1') FROM numbers(1)"]{assert!(parse_select(sql).is_err(),"{sql}");}
}