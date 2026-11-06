use super::*;
use crate::distributed_chunk_plan::ChunkRun;

fn run(first: usize, count: usize) -> ChunkRun {
    ChunkRun {
        first_index: first,
        chunk_count: count,
        offset: (first as u64).saturating_mul(7),
        length: (count as u64).saturating_mul(7),
    }
}

#[test]
fn completing_and_requeuing_runs_matches_scalar_bitmap_reference() {
    let runs = [run(0, 7), run(8, 64), run(129, 256), run(511, 513)];
    let mut batched = ResumeMap::new(1024).unwrap();
    let mut scalar = batched.clone();
    batched.mark_complete_runs(&runs).unwrap();
    for range in &runs {
        for index in range.first_index..range.first_index + range.chunk_count {
            scalar.mark_complete(index).unwrap();
        }
    }
    assert_eq!(batched, scalar);
    assert_eq!(
        batched.encode_binary_checked(),
        scalar.encode_binary_checked()
    );
    let invalidations = [run(4, 3), run(12, 24), run(1000, 24)];
    batched.invalidate_runs(&invalidations).unwrap();
    for range in &invalidations {
        scalar
            .invalidate_range(range.first_index, range.first_index + range.chunk_count)
            .unwrap();
    }
    assert_eq!(batched, scalar);
}

#[test]
fn invalid_late_chunk_run_cannot_partially_commit_progress() {
    let mut map = ResumeMap::new(1024).unwrap();
    map.mark_complete_range(100, 200).unwrap();
    let original = map.encode_binary_checked();
    let invalid = [
        vec![run(0, 8), run(8, 0)],
        vec![run(0, 8), run(7, 9)],
        vec![run(0, 8), run(1020, 5)],
        vec![run(0, 8), run(usize::MAX, 2)],
    ];
    for runs in &invalid {
        assert!(map.mark_complete_runs(runs).is_err());
        assert!(map.invalidate_runs(runs).is_err());
        assert_eq!(map.encode_binary_checked(), original);
    }
    assert!(map.mark_complete_runs(&[]).is_ok());
    assert_eq!(map.encode_binary_checked(), original);
}
