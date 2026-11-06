use super::*;

#[test]
fn counts_constant_predicates_without_per_row_dispatch() {
    let values = vec![NumericColumn::UInt64((0..100_000).collect())];
    for (value, expected) in [(true, 100_000), (false, 0)] {
        let branches = vec![vec![CompiledPredicate {
            index: 0,
            test: CompiledTest::Always(value),
        }]];
        assert_eq!(count_batch_matches(&values, &branches), expected);
    }
}
