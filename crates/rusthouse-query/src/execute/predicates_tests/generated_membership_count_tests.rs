use super::*;

#[test]
fn large_generated_membership_counts_only_the_list_values() {
    let small = vec![CompiledPredicate {
        index: 0,
        test: CompiledTest::In(vec![
            Number::Unsigned(1),
            Number::Unsigned(1),
            Number::Unsigned(99),
            Number::Unsigned(1_000_000_000),
        ]),
    }];
    assert_eq!(
        count_monotonic_numbers(&[small], 0, 1_000_000_000, 1),
        Some(2)
    );
    let set = (0..32).map(Number::Unsigned).collect();
    let large = vec![CompiledPredicate {
        index: 0,
        test: compile_in_test(set, true),
    }];
    assert_eq!(
        count_monotonic_numbers(&[large], 0, 1_000_000_000, 1),
        Some(999_999_968)
    );
}
