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
    let more_signed = TypedColumn::Numeric(NumericColumn::Int64(vec![4]));
    update_group_numeric(&mut aggregate, &more_signed, 0).unwrap();
    assert_eq!(aggregate.render(), "10");
}
