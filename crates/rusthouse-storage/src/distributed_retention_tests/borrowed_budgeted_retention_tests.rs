use super::*;

#[test]
fn sorted_generation_sizes_borrow_and_unsorted_sizes_sort() {
    let sorted = (1..=2048u64).map(|id| (id, id % 37)).collect::<Vec<_>>();
    let reversed = sorted.iter().copied().rev().collect::<Vec<_>>();
    assert!(matches!(
        ordered_generation_sizes(&sorted),
        Cow::Borrowed(_)
    ));
    assert!(matches!(ordered_generation_sizes(&reversed), Cow::Owned(_)));
    assert_eq!(
        ordered_generation_sizes(&reversed).as_ref(),
        sorted.as_slice()
    );
    for budget in [0, 1, 100, 10_000, u64::MAX] {
        for keep_latest in [1, 2, 512, 2048, usize::MAX] {
            assert_eq!(
                retention_candidates_with_byte_budget(
                    &sorted,
                    2048,
                    keep_latest,
                    &[1, 9, 37],
                    budget
                ),
                retention_candidates_with_byte_budget(
                    &reversed,
                    2048,
                    keep_latest,
                    &[1, 9, 37],
                    budget
                ),
            );
        }
    }
}

#[test]
fn already_sorted_duplicate_generations_still_fail_validation() {
    assert!(matches!(
        ordered_generation_sizes(&[(1, 1), (1, 2)]),
        Cow::Borrowed(_)
    ));
    assert!(retention_candidates_with_byte_budget(&[(1, 1), (1, 2)], 1, 1, &[], 100).is_err());
}
