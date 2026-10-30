use super::*;

#[test]
fn direct_grouped_sum_skips_nulls_and_preserves_signed_values() {
    let column = TypedColumn::NullableNumeric {
        values: NumericColumn::Int64(vec![-3, 17, 9]),
        nulls: vec![0, 1, 0],
    };
    let mut aggregate = AggregateState::new(AggregateFunction::Sum);
    for row in 0..column.len() {
        update_group_numeric(&mut aggregate, &column, row).unwrap();
    }
    assert_eq!(aggregate.render(), "6");
    let non_numeric = TypedColumn::Numeric(NumericColumn::UInt64(vec![4]));
    update_group_numeric(&mut aggregate, &non_numeric, 0).unwrap();
    assert_eq!(aggregate.render(), "10");
}
