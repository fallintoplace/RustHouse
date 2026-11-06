use super::*;

#[test]
fn native_nan_checks_preserve_distinct_and_nullable_semantics() {
    let column = TypedColumn::NullableNumeric {
        values: NumericColumn::Float32(vec![f32::NAN, -0.0, 1.0, f32::NAN]),
        nulls: vec![0, 0, 0, 1],
    };
    assert!(numeric_nan_at(&column, 0));
    assert!(!numeric_nan_at(&column, 1));
    assert!(!numeric_nan_at(&column, 3));
    let branches = vec![vec![Condition {
        index: 0,
        test: Test::DistinctFrom(Cell::Numeric(Number::Float(f64::NAN)), false),
    }]];
    let batch = [column];
    assert_eq!(
        (0..4)
            .map(|row| passes(&batch, row, &branches))
            .collect::<Vec<_>>(),
        [false, true, true, true]
    );
    let not_distinct = vec![vec![Condition {
        index: 0,
        test: Test::DistinctFrom(Cell::Numeric(Number::Float(f64::NAN)), true),
    }]];
    assert_eq!(
        (0..4)
            .map(|row| passes(&batch, row, &not_distinct))
            .collect::<Vec<_>>(),
        [true, false, false, false]
    );
}
