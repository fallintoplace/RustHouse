use super::{Cell, matches_comparison_at};
use crate::Operator;
use rusthouse_storage::{NumericColumn, TypedColumn};

#[test]
fn borrowed_numeric_comparisons_match_signed_unsigned_and_nulls() {
    let signed = TypedColumn::Numeric(NumericColumn::Int64(vec![-3, 0, 7]));
    assert!(matches_comparison_at(
        &signed,
        0,
        &Cell::Numeric(super::Number::Signed(-2)),
        Operator::Lt
    ));
    assert!(!matches_comparison_at(
        &signed,
        0,
        &Cell::Numeric(super::Number::Signed(-2)),
        Operator::Ge
    ));
    let nullable = TypedColumn::NullableNumeric {
        values: NumericColumn::UInt64(vec![4, 99, 7]),
        nulls: vec![0, 1, 0],
    };
    let target = Cell::Numeric(super::Number::Unsigned(4));
    assert!(matches_comparison_at(&nullable, 0, &target, Operator::Eq));
    assert!(!matches_comparison_at(&nullable, 1, &target, Operator::Ne));
    assert!(matches_comparison_at(&nullable, 2, &target, Operator::Gt));
}
