use super::*;

#[test]
fn distinct_and_not_distinct_match_sequence_membership() {
    let count = |target, negated| {
        count_monotonic_numbers(
            &[vec![CompiledPredicate {
                index: 0,
                test: CompiledTest::DistinctFrom {
                    target: Number::Unsigned(target),
                    negated,
                },
            }]],
            5,
            1_000_000_000,
            2,
        )
    };
    assert_eq!(count(7, true), Some(1));
    assert_eq!(count(7, false), Some(999_999_999));
    assert_eq!(count(8, true), Some(0));
    assert_eq!(count(8, false), Some(1_000_000_000));
}
