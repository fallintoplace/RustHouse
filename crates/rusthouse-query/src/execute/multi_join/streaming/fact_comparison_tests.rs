use super::super::super::Number;
use super::*;

#[test]
fn fact_comparison_prefilter_keeps_only_fact_columns_and_rejects_nulls() {
    let comparisons = [
        (
            0,
            crate::Operator::Gt,
            typed::Cell::Numeric(Number::Unsigned(10)),
        ),
        (
            2,
            crate::Operator::Eq,
            typed::Cell::Numeric(Number::Unsigned(7)),
        ),
    ];
    let fact_comparisons = fact_side_comparisons(&comparisons, 2);

    assert_eq!(fact_comparisons.len(), 1);
    assert!(passes_fact_comparisons(
        &[
            typed::Cell::Numeric(Number::Unsigned(11)),
            typed::Cell::Null,
        ],
        &fact_comparisons,
    ));
    assert!(!passes_fact_comparisons(
        &[
            typed::Cell::Numeric(Number::Unsigned(10)),
            typed::Cell::Null,
        ],
        &fact_comparisons,
    ));
    assert!(!passes_fact_comparisons(
        &[typed::Cell::Null, typed::Cell::Null],
        &fact_comparisons,
    ));
}
