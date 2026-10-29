use super::*;

#[test]
fn null_is_separate_from_zero_and_all_signed_boundary_keys() {
    let nullable = TypedColumn::NullableNumeric {
        values: NumericColumn::Int8(vec![0, -1, i8::MIN, i8::MAX, 0]),
        nulls: vec![1, 0, 0, 0, 0],
    };
    assert_eq!(
        (0..5).map(|index| narrow_group_bucket(&nullable, index, 256).unwrap()).collect::<Vec<_>>(),
        [256, 255, 128, 127, 0],
    );
    let nullable_16 = TypedColumn::NullableNumeric {
        values: NumericColumn::UInt16(vec![0, 0, u16::MAX, 42]),
        nulls: vec![0, 1, 0, 0],
    };
    assert_eq!(
        (0..4).map(|index| narrow_group_bucket(&nullable_16, index, 65_536).unwrap()).collect::<Vec<_>>(),
        [0, 65_536, 65_535, 42],
    );
}

#[test]
fn same_dense_bucket_reuses_group_state_for_nullable_sum() {
    let dimension = TypedColumn::NullableNumeric {
        values: NumericColumn::UInt8(vec![1, 1, 2, 0, 0, 1]),
        nulls: vec![0, 0, 0, 1, 1, 0],
    };
    let amounts = TypedColumn::Numeric(NumericColumn::UInt64(vec![10, 20, 30, 40, 50, 60]));
    let mut slots = vec![usize::MAX; 257];
    let mut groups = Vec::<(Cell, AggregateState)>::new();
    for row in 0..6 {
        let bucket = narrow_group_bucket(&dimension, row, 256).unwrap();
        if slots[bucket] == usize::MAX {
            slots[bucket] = groups.len();
            groups.push((typed::cell(&dimension, row), AggregateState::new(AggregateFunction::Sum)));
        }
        update_group_numeric(&mut groups[slots[bucket]].1, &amounts, row).unwrap();
    }
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[slots[1]].1.render(), "90");
    assert_eq!(groups[slots[2]].1.render(), "30");
    assert_eq!(groups[slots[256]].1.render(), "90");
    assert!(matches!(groups[slots[256]].0, Cell::Null));
}
