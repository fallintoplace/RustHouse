use rusthouse_query::{parse_select,ConstantValue,Projection};
fn value(sql:&str)->i64{match parse_select(sql).unwrap().projection{Projection::Literal(x)=>x,Projection::Constants(v) if v.len()==1=>match v[0].value{ConstantValue::Integer(x)=>x,ref other=>panic!("wrong value {other:?}")},other=>panic!("wrong projection {other:?}")}}
#[test]fn wraps_int32_conversions(){
assert_eq!(value("SELECT toUInt32(4294967295)"),4294967295);
assert_eq!(value("SELECT toUInt32(-1)"),4294967295);
assert_eq!(value("SELECT toUInt32(4294967296)"),0);
assert_eq!(value("SELECT toUInt32(4294967297)"),1);
}
#[test]fn validates_args(){for sql in ["SELECT toUInt32()","SELECT toUInt32('1')","SELECT toUInt32(1,2)","SELECT toUInt32(1) FROM numbers(1)"]{assert!(parse_select(sql).is_err(),"{sql}");}}
