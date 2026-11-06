use super::*;

#[test]
fn exhaustive_checkpoint_prefixes_match_missing_iterator() {
    for length in 0..=257 {
        for stride in [2, 3, 5, 7, 11, 64] {
            let mut state = ResumeMap::new(length).unwrap();
            for index in (0..length).step_by(stride) {
                state.mark_complete(index).unwrap();
            }
            let reference = state.remaining_indices().collect::<Vec<_>>();
            assert_eq!(state.nth_missing(reference.len()), None);
            for (rank, &chunk) in reference.iter().enumerate() {
                assert_eq!(
                    state.nth_missing(rank),
                    Some(chunk),
                    "length={length}, stride={stride}, rank={rank}"
                );
            }
            for end in 0..=length {
                assert_eq!(
                    state.missing_count_before(end).unwrap(),
                    reference.partition_point(|&index| index < end),
                    "length={length}, stride={stride}, end={end}",
                );
            }
            assert!(state.missing_count_before(length + 1).is_err());
        }
    }
}

#[test]
fn full_word_runs_partial_tail_and_large_sparse_state() {
    let mut state = ResumeMap::new(65_536).unwrap();
    state.mark_complete_range(0, 65_536).unwrap();
    assert!(state.nth_missing(0).is_none());
    state.invalidate_indices(&[0, 63, 64, 65_535]).unwrap();
    assert_eq!(state.remaining_count(), 4);
    assert_eq!(
        (0..5).map(|i| state.nth_missing(i)).collect::<Vec<_>>(),
        vec![Some(0), Some(63), Some(64), Some(65_535), None],
    );
    assert_eq!(state.missing_count_before(64).unwrap(), 2);
    assert_eq!(state.missing_count_before(65_536).unwrap(), 4);
    let mut tail = ResumeMap::new(65).unwrap();
    tail.mark_complete_range(0, 64).unwrap();
    assert_eq!(tail.nth_missing(0), Some(64));
    assert_eq!(tail.nth_missing(1), None);
}
