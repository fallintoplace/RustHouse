use rusthouse_query::{parse_select,ConstantValue,Projection};
fn value(sql:&str)->ConstantValue{match parse_select(sql).unwrap().projection{Projection::Constants(v) if v.len()==1=>v[0].value.clone(),Projection::Literal(n)=>ConstantValue::Integer(n),other=>panic!("unexpected {other:?}")}}
#[test]fn converts_numeric_strings_and_wraps(){
assert_eq!(value("SELECT toUInt8OrNull('255')"),ConstantValue::Integer(255));
assert_eq!(value("SELECT toUInt8OrNull('256')"),ConstantValue::Integer(0));
assert_eq!(value("SELECT toUInt8OrNull('4294967295')"),ConstantValue::Integer(255));
}
#[test]fn returns_null_for_invalid_inputs(){
 for sql in ["SELECT toUInt8OrNull('bad')","SELECT toUInt8OrNull('1.5')","SELECT toUInt8OrNull('0x12')","SELECT toUInt8OrNull(NULL)","SELECT toUInt8OrNull('-1')"]{assert_eq!(value(sql),ConstantValue::Null,"{sql}");}
}
#[test]fn rejects_wrong_arguments(){
 for sql in ["SELECT toUInt8OrNull()","SELECT toUInt8OrNull(123)","SELECT toUInt8OrNull('1','2')","SELECT toUInt8OrNull('1') FROM numbers(1)"]{assert!(parse_select(sql).is_err(),"{sql}");}
}
