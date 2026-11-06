use super::*;

#[test]
fn inline_batch_matches_individual_updates_across_bitmap_boundaries() {
    for chunks in [0, 1, 7, 8, 9, 63, 64, 65, 127, 65_536] {
        let indexes = (0..chunks).step_by(7).take(16).collect::<Vec<_>>();
        let mut batch = ResumeMap::new(chunks).unwrap();
        let mut scalar = ResumeMap::new(chunks).unwrap();
        batch.mark_complete_unordered(&indexes).unwrap();
        for &index in &indexes {
            scalar.mark_complete(index).unwrap();
        }
        assert_eq!(batch, scalar);
        batch.invalidate_unordered(&indexes).unwrap();
        assert_eq!(batch, ResumeMap::new(chunks).unwrap());
    }
}

#[test]
fn invalid_inline_batch_never_changes_a_verified_checkpoint() {
    let mut progress = ResumeMap::new(65_536).unwrap();
    progress.mark_complete_indices(&[0, 8, 64, 65_535]).unwrap();
    let original = progress.clone();
    assert!(progress.mark_complete_unordered(&[10, 20, 10]).is_err());
    assert_eq!(progress, original);
    assert!(progress.invalidate_unordered(&[8, 65_536]).is_err());
    assert_eq!(progress, original);
    assert!(progress.invalidate_unordered(&[8, 8]).is_err());
    assert_eq!(progress, original);
}
