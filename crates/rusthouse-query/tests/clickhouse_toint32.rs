use rusthouse_query::{parse_select,ConstantValue,Projection};
fn value(sql:&str)->i64{match parse_select(sql).unwrap().projection{Projection::Literal(x)=>x,Projection::Constants(v) if v.len()==1=>match v[0].value{ConstantValue::Integer(x)=>x,ref other=>panic!("wrong value {other:?}")},other=>panic!("wrong projection {other:?}")}}
#[test] fn wraps_signed_width() {
    assert_eq!(value("SELECT toInt32(2147483647)"),2147483647);
    assert_eq!(value("SELECT toInt32(2147483648)"),-2147483648);
    assert_eq!(value("SELECT toInt32(-2147483649)"),2147483647);
}
#[test] fn rejects_wrong_inputs() {
    for sql in ["SELECT toInt32()","SELECT toInt32('1')","SELECT toInt32(1,2)","SELECT toInt32(1) FROM numbers(1)"] {
        assert!(parse_select(sql).is_err(),"{sql}");
    }
}