use super::*;

#[test]
fn compiled_short_bitmap_preserves_negative_values_and_nulls() {
    let values = (-20..0).map(|value| value.to_string()).collect::<Vec<_>>();
    let includes = [vec![Condition {
        index: 0,
        test: compile_in("Nullable(Int16)", &values, false).unwrap(),
    }]];
    assert!(matches!(&includes[0][0].test, Test::ShortInSet(_, false)));
    let batch = [TypedColumn::NullableNumeric {
        values: NumericColumn::Int16(vec![-20, -1, 100, -5]),
        nulls: vec![0, 0, 0, 1],
    }];
    assert!(passes(&batch, 0, &includes));
    assert!(passes(&batch, 1, &includes));
    assert!(!passes(&batch, 2, &includes));
    assert!(!passes(&batch, 3, &includes));
    let unsigned = (0..100).map(|value| value.to_string()).collect::<Vec<_>>();
    let excludes = [vec![Condition {
        index: 0,
        test: compile_in("UInt16", &unsigned, true).unwrap(),
    }]];
    let batch = [TypedColumn::Numeric(NumericColumn::UInt16(vec![
        0, 99, 65535,
    ]))];
    assert!(!passes(&batch, 0, &excludes));
    assert!(!passes(&batch, 1, &excludes));
    assert!(passes(&batch, 2, &excludes));
}
