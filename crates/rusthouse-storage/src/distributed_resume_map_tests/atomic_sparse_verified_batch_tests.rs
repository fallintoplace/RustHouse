use super::*;

#[test]
fn sparse_batch_matches_single_chunk_updates_at_bitmap_boundaries() {
    for chunks in [0, 1, 7, 8, 9, 16, 65_535, 65_536] {
        let indexes = (0..chunks).step_by(3).collect::<Vec<_>>();
        let mut batched = ResumeMap::new(chunks).unwrap();
        let mut sequential = batched.clone();
        batched.mark_complete_indices(&indexes).unwrap();
        for &index in &indexes {
            sequential.mark_complete(index).unwrap();
        }
        assert_eq!(batched, sequential);
        assert_eq!(batched.completed_count(), indexes.len());
        batched
            .invalidate_indices(
                &indexes
                    .iter()
                    .copied()
                    .filter(|x| x % 2 == 0)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        for &index in indexes.iter().filter(|&&x| x % 2 == 0) {
            sequential.invalidate_range(index, index + 1).unwrap();
        }
        assert_eq!(batched, sequential);
        assert_eq!(
            ResumeMap::decode_binary(&batched.encode_binary()).unwrap(),
            sequential
        );
    }
}

#[test]
fn an_invalid_late_index_never_commits_earlier_bits() {
    let mut progress = ResumeMap::new(17).unwrap();
    progress.mark_complete(7).unwrap();
    let baseline = progress.clone();
    for invalid in [vec![0, 1, 17], vec![0, 3, 3], vec![0, 7, 6]] {
        assert!(progress.mark_complete_indices(&invalid).is_err());
        assert_eq!(progress, baseline);
        assert!(progress.invalidate_indices(&invalid).is_err());
        assert_eq!(progress, baseline);
    }
    progress.mark_complete_indices(&[]).unwrap();
    progress.invalidate_indices(&[]).unwrap();
    assert_eq!(progress, baseline);
}
