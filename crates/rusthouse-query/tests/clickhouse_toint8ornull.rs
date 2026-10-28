use rusthouse_query::{parse_select,ConstantValue,Projection};
fn value(sql:&str)->ConstantValue{match parse_select(sql).unwrap().projection{Projection::Constants(v) if v.len()==1=>v[0].value.clone(),Projection::Literal(n)=>ConstantValue::Integer(n),other=>panic!("unexpected {other:?}")}}
#[test]fn converts_numeric_strings_and_wraps(){
assert_eq!(value("SELECT toInt8OrNull('127')"),ConstantValue::Integer(127));
assert_eq!(value("SELECT toInt8OrNull('128')"),ConstantValue::Integer(-128));
assert_eq!(value("SELECT toInt8OrNull('-129')"),ConstantValue::Integer(127));
}
#[test]fn returns_null_for_invalid_inputs(){
 for sql in ["SELECT toInt8OrNull('bad')","SELECT toInt8OrNull('1.5')","SELECT toInt8OrNull('0x12')","SELECT toInt8OrNull(NULL)"]{assert_eq!(value(sql),ConstantValue::Null,"{sql}");}
}
#[test]fn rejects_wrong_arguments(){
 for sql in ["SELECT toInt8OrNull()","SELECT toInt8OrNull(123)","SELECT toInt8OrNull('1','2')","SELECT toInt8OrNull('1') FROM numbers(1)"]{assert!(parse_select(sql).is_err(),"{sql}");}
}
