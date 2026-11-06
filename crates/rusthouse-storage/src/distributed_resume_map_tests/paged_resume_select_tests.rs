use super::*;

#[test]
fn rank_select_from_every_cursor_matches_missing_iterator() {
    for length in [0, 1, 7, 8, 9, 63, 64, 65, 127, 128, 129, 257] {
        for stride in [2, 3, 5, 7] {
            let mut state = ResumeMap::new(length).unwrap();
            for index in (0..length).step_by(stride) {
                state.mark_complete(index).unwrap();
            }
            for start in 0..=length {
                let missing = state.remaining_indices_from(start).collect::<Vec<_>>();
                for (rank, &index) in missing.iter().enumerate() {
                    assert_eq!(
                        state.nth_missing_from(start, rank).unwrap(),
                        Some(index),
                        "length={length}, stride={stride}, start={start}, rank={rank}"
                    );
                }
                assert_eq!(state.nth_missing_from(start, missing.len()), Ok(None));
                assert_eq!(state.nth_missing_from(start, usize::MAX), Ok(None));
            }
            assert!(state.nth_missing_from(length + 1, 0).is_err());
        }
    }
}

#[test]
fn large_checkpoint_selects_only_remaining_holes_past_cursor() {
    let mut state = ResumeMap::new(65_535).unwrap();
    state.mark_complete_range(0, 65_535).unwrap();
    state.invalidate_indices(&[0, 63, 64, 65_534]).unwrap();
    assert_eq!(state.nth_missing_from(1, 0), Ok(Some(63)));
    assert_eq!(state.nth_missing_from(64, 0), Ok(Some(64)));
    assert_eq!(state.nth_missing_from(65, 0), Ok(Some(65_534)));
    assert_eq!(state.nth_missing_from(65_534, 1), Ok(None));
}
