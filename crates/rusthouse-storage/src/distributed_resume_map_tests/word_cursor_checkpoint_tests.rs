use super::*;

#[test]
fn word_aligned_and_unaligned_pages_match_full_iteration() {
    for chunks in [
        0usize, 1, 7, 8, 9, 63, 64, 65, 127, 128, 129, 255, 256, 257, 8193,
    ] {
        for stride in [1usize, 2, 5, 31, 64] {
            let mut map = ResumeMap::new(chunks).unwrap();
            for index in (0..chunks).step_by(stride) {
                map.mark_complete(index).unwrap();
            }
            let complete = map.remaining_indices().collect::<Vec<_>>();
            let mut cursors = vec![0, chunks / 2, chunks];
            cursors.extend([1, 7, 8, 31, 63, 64, 65, 127, 128, 129, 255, 256, 257]);
            for start in cursors {
                let actual = map.remaining_indices_from(start).collect::<Vec<_>>();
                let expected = complete
                    .iter()
                    .copied()
                    .filter(|&index| index >= start)
                    .collect::<Vec<_>>();
                assert_eq!(
                    actual, expected,
                    "chunks={chunks} stride={stride} start={start}"
                );
            }
        }
    }
}

#[test]
fn dense_large_checkpoint_keeps_only_final_unverified_indices() {
    let mut map = ResumeMap::new(65_535).unwrap();
    map.mark_complete_range(0, 65_530).unwrap();
    let expected = (65_530..65_535).collect::<Vec<_>>();
    assert_eq!(map.remaining_indices_from(0).collect::<Vec<_>>(), expected);
    assert_eq!(
        map.remaining_indices_from(65_532).collect::<Vec<_>>(),
        vec![65_532, 65_533, 65_534]
    );
    assert_eq!(map.remaining_indices_from(65_535).count(), 0);
    assert_eq!(map.remaining_indices_from(usize::MAX).count(), 0);
}
