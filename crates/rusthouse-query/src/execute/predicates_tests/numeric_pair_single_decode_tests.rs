use super::*;

#[test]
fn paired_bounds_keep_inclusive_edges_and_or_branches() {
    let batch = [NumericColumn::UInt16(vec![999, 1000, 5000, 9000, 9001])];
    let range = vec![
        CompiledPredicate {
            index: 0,
            test: CompiledTest::Comparison {
                operator: Operator::Ge,
                target: Number::Unsigned(1000),
            },
        },
        CompiledPredicate {
            index: 0,
            test: CompiledTest::Comparison {
                operator: Operator::Le,
                target: Number::Unsigned(9000),
            },
        },
    ];
    let branches = vec![range];
    assert_eq!(
        (0..5)
            .map(|row| batch_matches(&batch, row, &branches))
            .collect::<Vec<_>>(),
        vec![false, true, true, true, false]
    );
    assert_eq!(count_batch_matches(&batch, &branches), 3);
}

#[test]
fn paired_float_comparisons_leave_nan_unmatched() {
    let batch = [NumericColumn::Float64(vec![f64::NAN, -1.0, 0.0, 2.0])];
    let branches = vec![vec![
        CompiledPredicate {
            index: 0,
            test: CompiledTest::Comparison {
                operator: Operator::Ge,
                target: Number::Float(-1.0),
            },
        },
        CompiledPredicate {
            index: 0,
            test: CompiledTest::Comparison {
                operator: Operator::Le,
                target: Number::Float(1.0),
            },
        },
    ]];
    assert_eq!(
        (0..4)
            .map(|row| batch_matches(&batch, row, &branches))
            .collect::<Vec<_>>(),
        vec![false, true, true, false]
    );
}
