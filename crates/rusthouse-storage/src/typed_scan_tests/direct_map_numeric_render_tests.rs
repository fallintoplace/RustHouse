use super::*;

#[test]
fn renders_large_numeric_array_without_intermediate_strings() {
    let values = NumericColumn::UInt64((0..10_000u64).collect());
    let mut row = String::from("[");
    for index in 0..10_000 {
        if index != 0 {
            row.push(',');
        }
        append_map_numeric(&mut row, &values, index);
    }
    row.push(']');
    assert!(row.starts_with("[0,1,2,3,4,5,6,7,8,9,"));
    assert!(row.ends_with(",9997,9998,9999]"));
    assert_eq!(row.bytes().filter(|&byte| byte == b',').count(), 9_999);
}

#[test]
fn appends_signed_unsigned_and_float_map_values_in_place() {
    let mut text = String::from("{");
    append_map_numeric(&mut text, &NumericColumn::UInt128(vec![u128::MAX]), 0);
    text.push(',');
    append_map_numeric(&mut text, &NumericColumn::Int64(vec![-1234]), 0);
    text.push(',');
    append_map_numeric(&mut text, &NumericColumn::Float32(vec![1.5]), 0);
    text.push('}');
    assert_eq!(text, format!("{{{},-1234,1.5}}", u128::MAX));
}
