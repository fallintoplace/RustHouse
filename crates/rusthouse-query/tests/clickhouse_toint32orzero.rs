use rusthouse_query::{parse_select,ConstantValue,Projection};
fn value(sql:&str)->ConstantValue{match parse_select(sql).unwrap().projection{Projection::Constants(v) if v.len()==1=>v[0].value.clone(),Projection::Literal(n)=>ConstantValue::Integer(n),other=>panic!("unexpected {other:?}")}}
#[test]fn converts_numeric_text_and_errors(){
assert_eq!(value("SELECT toInt32OrZero('2147483647')"),ConstantValue::Integer(2147483647));
assert_eq!(value("SELECT toInt32OrZero('2147483648')"),ConstantValue::Integer(-2147483648));
assert_eq!(value("SELECT toInt32OrZero('-2147483649')"),ConstantValue::Integer(2147483647));
assert_eq!(value("SELECT toInt32OrZero('bad')"),ConstantValue::Integer(0));
assert_eq!(value("SELECT toInt32OrZero('1.25')"),ConstantValue::Integer(0));
assert_eq!(value("SELECT toInt32OrZero(NULL)"),ConstantValue::Null);
}
#[test]fn rejects_invalid_argument_shapes(){for sql in ["SELECT toInt32OrZero()","SELECT toInt32OrZero(2)","SELECT toInt32OrZero('1','2')","SELECT toInt32OrZero('1') FROM numbers(1)"]{assert!(parse_select(sql).is_err(),"{sql}");}}
