use super::{Condition, Test, passes};
use super::{Number, NumberKey};
use rusthouse_storage::{NumericColumn, TypedColumn};
use std::collections::HashSet;

#[test]
fn native_numeric_membership_respects_null_nan_and_negation() {
    let targets = HashSet::from([NumberKey::from(Number::Signed(-3))]);
    let batch = [TypedColumn::NullableNumeric {
        values: NumericColumn::Int64(vec![-3, 2, -3]),
        nulls: vec![0, 1, 0],
    }];
    let includes = [vec![Condition {
        index: 0,
        test: Test::NumericInSet(HashSet::from([NumberKey::from(Number::Signed(-3))]), false),
    }]];
    let excludes = [vec![Condition {
        index: 0,
        test: Test::NumericInSet(targets, true),
    }]];
    assert!(passes(&batch, 0, &includes));
    assert!(!passes(&batch, 1, &includes));
    assert!(!passes(&batch, 1, &excludes));
    assert!(!passes(&batch, 2, &excludes));
    let floats = [TypedColumn::Numeric(NumericColumn::Float64(vec![f64::NAN]))];
    let float_set = [vec![Condition {
        index: 0,
        test: Test::NumericInSet(
            HashSet::from([NumberKey::from(Number::Float(f64::NAN))]),
            false,
        ),
    }]];
    assert!(!passes(&floats, 0, &float_set));
}
