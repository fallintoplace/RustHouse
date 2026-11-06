use super::*;

#[test]
fn constant_predicates_do_not_iterate_billions_of_rows() {
    for (value, expected) in [(true, 1_000_000_000), (false, 0)] {
        assert_eq!(
            count_monotonic_numbers(
                &[vec![CompiledPredicate {
                    index: 0,
                    test: CompiledTest::Always(value),
                }]],
                0,
                1_000_000_000,
                1
            ),
            Some(expected),
        );
    }
}
