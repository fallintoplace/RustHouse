use super::*;

#[test]
fn rejects_noncanonical_and_mismatched_nullable_bitmaps() {
    assert!(
        wrap_null_map(
            TypedColumn::Numeric(NumericColumn::UInt8(vec![1, 2])),
            Some(vec![0, 2]),
        )
        .is_err()
    );
    assert!(
        wrap_null_map(
            TypedColumn::Numeric(NumericColumn::UInt8(vec![1, 2])),
            Some(vec![0]),
        )
        .is_err()
    );
    let result = wrap_null_map(
        TypedColumn::Numeric(NumericColumn::UInt8(vec![1, 2])),
        Some(vec![0, 1]),
    )
    .unwrap();
    assert!(matches!(
        result,
        TypedColumn::NullableNumeric { nulls, .. } if nulls == vec![0, 1]
    ));
}
