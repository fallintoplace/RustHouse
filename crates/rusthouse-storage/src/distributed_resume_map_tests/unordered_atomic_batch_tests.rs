use super::*;

#[test]
fn unordered_batches_match_individual_updates_across_bitmap_boundaries() {
    for chunks in [0usize, 1, 7, 8, 9, 63, 64, 65, 257, 65_536] {
        let mut map = ResumeMap::new(chunks).unwrap();
        let indexes: Vec<usize> = (0..chunks).rev().filter(|i| i % 3 == 0).collect();
        let mut reference = ResumeMap::new(chunks).unwrap();
        for &index in &indexes {
            reference.mark_complete(index).unwrap();
        }
        map.mark_complete_unordered(&indexes).unwrap();
        assert_eq!(map, reference, "chunks={chunks}");
        map.invalidate_unordered(&indexes).unwrap();
        assert_eq!(map.completed_count(), 0);
        assert_eq!(map.remaining_count(), chunks);
    }
}

#[test]
fn failed_unordered_updates_are_atomic_even_after_valid_indices() {
    let mut checkpoint = ResumeMap::new(130).unwrap();
    checkpoint.mark_complete_range(0, 70).unwrap();
    let original = checkpoint.clone();
    for bad in [
        vec![100, 1, 100],
        vec![3, 129, 130],
        vec![129, 0, usize::MAX],
    ] {
        assert!(checkpoint.mark_complete_unordered(&bad).is_err());
        assert_eq!(checkpoint, original);
        assert!(checkpoint.invalidate_unordered(&bad).is_err());
        assert_eq!(checkpoint, original);
    }
    checkpoint
        .mark_complete_unordered(&[129, 128, 71, 70])
        .unwrap();
    assert_eq!(checkpoint.completed_count(), 74);
    checkpoint.invalidate_unordered(&[71, 129, 0]).unwrap();
    assert_eq!(checkpoint.completed_count(), 71);
}
