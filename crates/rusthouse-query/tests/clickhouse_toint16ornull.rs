use rusthouse_query::{parse_select,ConstantValue,Projection};
fn value(sql:&str)->ConstantValue{match parse_select(sql).unwrap().projection{Projection::Constants(v) if v.len()==1=>v[0].value.clone(),Projection::Literal(n)=>ConstantValue::Integer(n),other=>panic!("unexpected {other:?}")}}
#[test]fn converts_numeric_strings_and_wraps(){
assert_eq!(value("SELECT toInt16OrNull('32767')"),ConstantValue::Integer(32767));
assert_eq!(value("SELECT toInt16OrNull('32768')"),ConstantValue::Integer(-32768));
assert_eq!(value("SELECT toInt16OrNull('-32769')"),ConstantValue::Integer(32767));
}
#[test]fn returns_null_for_invalid_inputs(){
 for sql in ["SELECT toInt16OrNull('bad')","SELECT toInt16OrNull('1.5')","SELECT toInt16OrNull('0x12')","SELECT toInt16OrNull(NULL)"]{assert_eq!(value(sql),ConstantValue::Null,"{sql}");}
}
#[test]fn rejects_wrong_arguments(){
 for sql in ["SELECT toInt16OrNull()","SELECT toInt16OrNull(123)","SELECT toInt16OrNull('1','2')","SELECT toInt16OrNull('1') FROM numbers(1)"]{assert!(parse_select(sql).is_err(),"{sql}");}
}
