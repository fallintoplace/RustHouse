use super::*;

#[test]
fn sorted_dense_and_sparse_batches_match_single_bit_transitions() {
    for chunks in [
        0usize, 1, 7, 8, 31, 32, 33, 63, 64, 65, 255, 256, 257, 8_193, 65_535, 65_536,
    ] {
        let mut batched = ResumeMap::new(chunks).unwrap();
        let mut scalar = ResumeMap::new(chunks).unwrap();
        let positions = (0..chunks)
            .filter(|index| index % 11 != 0 && index % 17 != 0)
            .collect::<Vec<_>>();
        batched.mark_complete_indices(&positions).unwrap();
        for &index in &positions {
            scalar.mark_complete(index).unwrap();
        }
        assert_eq!(batched, scalar, "completed chunks={chunks}");
        let invalidations = positions
            .iter()
            .copied()
            .filter(|index| index % 3 != 0)
            .collect::<Vec<_>>();
        batched.invalidate_indices(&invalidations).unwrap();
        for &index in &invalidations {
            scalar.invalidate_range(index, index + 1).unwrap();
        }
        assert_eq!(batched, scalar, "invalidated chunks={chunks}");
        assert_eq!(batched.remaining(), scalar.remaining());
        assert_eq!(
            batched.encode_binary_checked(),
            scalar.encode_binary_checked()
        );
    }
}

#[test]
fn dense_late_index_validation_never_commits_any_prior_byte() {
    let mut map = ResumeMap::new(1024).unwrap();
    map.mark_complete_range(0, 200).unwrap();
    let original = map.clone();
    let mut invalid = (0..100usize).collect::<Vec<_>>();
    invalid.push(1024);
    assert!(map.mark_complete_indices(&invalid).is_err());
    assert_eq!(map, original);
    assert!(map.invalidate_indices(&invalid).is_err());
    assert_eq!(map, original);
    invalid.pop();
    invalid.push(99);
    assert!(map.mark_complete_indices(&invalid).is_err());
    assert_eq!(map, original);
}
