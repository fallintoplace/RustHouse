use super::*;

#[test]
fn failed_chunk_range_is_requeued_without_losing_verified_neighbors() {
    let mut map = ResumeMap::new(17).unwrap();
    map.mark_complete_range(0, 17).unwrap();
    assert!(map.complete());
    map.invalidate_range(5, 13).unwrap();
    assert_eq!(map.remaining(), (5..13).collect::<Vec<_>>());
    assert_eq!(map.completed_count(), 9);
    let restored = ResumeMap::decode_binary(&map.encode_binary()).unwrap();
    assert_eq!(restored, map);
    let mut restored = restored;
    restored.mark_complete_range(5, 13).unwrap();
    assert!(restored.complete());
}

#[test]
fn invalidation_is_exact_across_byte_boundaries() {
    for len in [0usize, 1, 7, 8, 9, 16, 17, 31, 63, 64, 65, 1024, 65_536] {
        let all = (0..len).filter(|index| index % 3 != 2).collect::<Vec<_>>();
        for &(start, end) in &[(0, 0), (0, len), (len / 3, len / 2), (len, len)] {
            let mut progress = ResumeMap::new(len).unwrap();
            for &index in &all {
                progress.mark_complete(index).unwrap();
            }
            progress.invalidate_range(start, end).unwrap();
            let expected = (0..len)
                .filter(|&index| index % 3 == 2 || (start..end).contains(&index))
                .collect::<Vec<_>>();
            assert_eq!(progress.remaining(), expected, "{len} {start} {end}");
        }
    }
}

#[test]
fn out_of_bounds_invalidation_never_changes_checkpoint() {
    let mut map = ResumeMap::new(9).unwrap();
    map.mark_complete_range(0, 9).unwrap();
    let original = map.encode_binary();
    assert!(map.invalidate_range(10, 10).is_err());
    assert!(map.invalidate_range(7, 6).is_err());
    assert!(map.invalidate_range(0, 10).is_err());
    assert_eq!(map.encode_binary(), original);
}
