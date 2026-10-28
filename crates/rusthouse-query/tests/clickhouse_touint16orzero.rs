use rusthouse_query::{parse_select,ConstantValue,Projection};
fn value(sql:&str)->ConstantValue{match parse_select(sql).unwrap().projection{Projection::Constants(v) if v.len()==1=>v[0].value.clone(),Projection::Literal(n)=>ConstantValue::Integer(n),other=>panic!("unexpected {other:?}")}}
#[test]fn converts_numeric_text_and_errors(){
assert_eq!(value("SELECT toUInt16OrZero('65535')"),ConstantValue::Integer(65535));
assert_eq!(value("SELECT toUInt16OrZero('65536')"),ConstantValue::Integer(0));
assert_eq!(value("SELECT toUInt16OrZero('65537')"),ConstantValue::Integer(1));
assert_eq!(value("SELECT toUInt16OrZero('-1')"),ConstantValue::Integer(0));
assert_eq!(value("SELECT toUInt16OrZero('bad')"),ConstantValue::Integer(0));
assert_eq!(value("SELECT toUInt16OrZero(NULL)"),ConstantValue::Null);
}
#[test]fn rejects_invalid_argument_shapes(){for sql in ["SELECT toUInt16OrZero()","SELECT toUInt16OrZero(2)","SELECT toUInt16OrZero('1','2')","SELECT toUInt16OrZero('1') FROM numbers(1)"]{assert!(parse_select(sql).is_err(),"{sql}");}}
