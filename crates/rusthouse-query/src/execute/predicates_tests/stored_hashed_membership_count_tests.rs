use super::*;

#[test]
fn hashed_membership_matches_row_by_row_for_wide_integer_columns() {
    let signed = vec![NumericColumn::Int64((-5_000..5_000).collect())];
    let unsigned = vec![NumericColumn::UInt64((0..10_000).collect())];
    for (batch, signed) in [(&signed, true), (&unsigned, false)] {
        for negated in [false, true] {
            let values: Vec<Number> = (0..32)
                .map(|index| {
                    if signed {
                        Number::Signed(index - 12)
                    } else {
                        Number::Unsigned(index as u128)
                    }
                })
                .collect();
            let branches = vec![vec![CompiledPredicate {
                index: 0,
                test: compile_in_test(values, negated),
            }]];
            let expected = (0..batch[0].len())
                .filter(|&row| batch_matches(batch, row, &branches))
                .count();
            assert_eq!(count_batch_matches(batch, &branches), expected);
        }
    }
}
