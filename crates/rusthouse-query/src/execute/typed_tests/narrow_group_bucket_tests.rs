use super::*;

#[test]
fn dense_numeric_buckets_preserve_exact_signed_values() {
    let signed8 = TypedColumn::Numeric(NumericColumn::Int8(vec![i8::MIN, -1, 0, i8::MAX]));
    assert_eq!(
        (0..4)
            .map(|row| dense_group_bucket(&signed8, row))
            .collect::<Vec<_>>(),
        [Some(128), Some(255), Some(0), Some(127)]
    );
    let signed16 = TypedColumn::Numeric(NumericColumn::Int16(vec![i16::MIN, -1, 0, i16::MAX]));
    assert_eq!(
        (0..4)
            .map(|row| dense_group_bucket(&signed16, row))
            .collect::<Vec<_>>(),
        [Some(32768), Some(65535), Some(0), Some(32767)]
    );
}

#[test]
fn nullable_and_wider_dimensions_never_share_group_ids() {
    let nullable = TypedColumn::NullableNumeric {
        values: NumericColumn::UInt16(vec![0, 0, u16::MAX, 1]),
        nulls: vec![0, 1, 0, 0],
    };
    assert_eq!(
        (0..4)
            .map(|row| dense_group_bucket(&nullable, row))
            .collect::<Vec<_>>(),
        [Some(0), None, Some(65535), Some(1)]
    );
    let wide = TypedColumn::Numeric(NumericColumn::UInt32(vec![1]));
    assert_eq!(dense_group_bucket(&wide, 0), None);
}
