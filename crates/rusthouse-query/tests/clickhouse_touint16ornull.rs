use rusthouse_query::{parse_select,ConstantValue,Projection};
fn value(sql:&str)->ConstantValue{match parse_select(sql).unwrap().projection{Projection::Constants(v) if v.len()==1=>v[0].value.clone(),Projection::Literal(n)=>ConstantValue::Integer(n),other=>panic!("unexpected {other:?}")}}
#[test]fn converts_numeric_strings_and_wraps(){
assert_eq!(value("SELECT toUInt16OrNull('65535')"),ConstantValue::Integer(65535));
assert_eq!(value("SELECT toUInt16OrNull('65536')"),ConstantValue::Integer(0));
assert_eq!(value("SELECT toUInt16OrNull('65537')"),ConstantValue::Integer(1));
}
#[test]fn returns_null_for_invalid_inputs(){
 for sql in ["SELECT toUInt16OrNull('bad')","SELECT toUInt16OrNull('1.5')","SELECT toUInt16OrNull('0x12')","SELECT toUInt16OrNull(NULL)","SELECT toUInt16OrNull('-1')"]{assert_eq!(value(sql),ConstantValue::Null,"{sql}");}
}
#[test]fn rejects_wrong_arguments(){
 for sql in ["SELECT toUInt16OrNull()","SELECT toUInt16OrNull(123)","SELECT toUInt16OrNull('1','2')","SELECT toUInt16OrNull('1') FROM numbers(1)"]{assert!(parse_select(sql).is_err(),"{sql}");}
}
