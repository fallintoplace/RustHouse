use super::*;

#[test]
fn clamped_windows_return_partial_final_pages() {
    let all = plan_chunks(10, 3, 4).unwrap();
    let first = plan_chunk_window_clamped(10, 3, 4, 0, 3).unwrap();
    let tail = plan_chunk_window_clamped(10, 3, 4, 3, 3).unwrap();
    assert_eq!(first.len(), 3);
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].chunk, all[3]);
    assert!(
        plan_chunk_window_clamped(10, 3, 4, 4, usize::MAX)
            .unwrap()
            .is_empty()
    );
    assert!(plan_chunk_window_clamped(10, 3, 4, 5, 1).is_err());
    assert!(plan_chunk_window(10, 3, 4, 3, 2).is_err());
}

#[test]
fn partitions_without_gaps_or_overlaps() {
    assert_eq!(
        plan_chunks(10, 4, 3).unwrap(),
        vec![
            Chunk {
                offset: 0,
                length: 4
            },
            Chunk {
                offset: 4,
                length: 4
            },
            Chunk {
                offset: 8,
                length: 2
            },
        ]
    );
    assert!(plan_chunks(0, 4, 1).unwrap().is_empty());
    assert!(plan_chunks(10, 4, 2).is_err());
    assert!(plan_chunks(10, 0, 10).is_err());
}

#[test]
fn schedules_only_missing_chunks_with_stable_indices_and_byte_budget() {
    let pending = plan_missing_chunks(10, 4, 3, &[0, 2], 6, 3).unwrap();
    assert_eq!(
        pending,
        vec![
            IndexedChunk {
                index: 0,
                chunk: Chunk {
                    offset: 0,
                    length: 4
                }
            },
            IndexedChunk {
                index: 2,
                chunk: Chunk {
                    offset: 8,
                    length: 2
                }
            },
        ]
    );
    assert_eq!(
        plan_missing_chunks(10, 4, 3, &[0, 1, 2], 7, 3).unwrap(),
        vec![
            IndexedChunk {
                index: 0,
                chunk: Chunk {
                    offset: 0,
                    length: 4
                }
            },
            IndexedChunk {
                index: 2,
                chunk: Chunk {
                    offset: 8,
                    length: 2
                }
            },
        ],
    );
    assert_eq!(
        plan_missing_chunks(10, 4, 3, &[2], 2, 1).unwrap(),
        vec![IndexedChunk {
            index: 2,
            chunk: Chunk {
                offset: 8,
                length: 2
            }
        }],
    );
    assert!(plan_missing_chunks(0, 4, 3, &[], 0, 3).unwrap().is_empty());
    assert!(
        plan_missing_chunks(10, 4, 3, &[2], 0, 3)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn zero_budget_still_validates_indexes_before_skipping_work() {
    let indexes = (0..65_536).collect::<Vec<_>>();
    assert!(
        plan_missing_chunks(65_536, 1, 65_536, &indexes, 0, 65_536)
            .unwrap()
            .is_empty()
    );
    assert!(plan_missing_chunks(4, 1, 4, &[3, 3], 0, 0).is_err());
    assert!(plan_missing_chunks(4, 1, 4, &[3, 4], 0, 2).is_err());
    assert!(plan_missing_chunks(4, 1, 4, &[3], 4, 0).unwrap().is_empty());
}

#[test]
fn tiny_resume_budget_does_not_reserve_full_batch() {
    let remaining = (0..65_536).collect::<Vec<_>>();
    let chunks = plan_missing_chunks(65_536, 1, 65_536, &remaining, 0, 65_536).unwrap();
    assert!(chunks.is_empty());
    assert!(chunks.capacity() <= 1);
    let last = plan_missing_chunks(9, 4, 3, &[2], 1, 3).unwrap();
    assert_eq!(last.len(), 1);
}

#[test]
fn validates_sparse_indexes_before_selecting_bounded_transfer_prefix() {
    let selected = plan_missing_chunks(101, 1, 128, &[0, 2, 7, 63, 64, 100], 3, 3).unwrap();
    assert_eq!(
        selected.iter().map(|part| part.index).collect::<Vec<_>>(),
        vec![0, 2, 7]
    );
    assert!(plan_missing_chunks(101, 1, 128, &[0, 2, 7, 7], 4, 4).is_err());
    assert!(plan_missing_chunks(101, 1, 128, &[0, 2, 101], 4, 4).is_err());
}

#[test]
fn rejects_corrupt_missing_chunk_indexes() {
    for indexes in [&[0, 0][..], &[2, 1][..], &[3][..]] {
        assert!(plan_missing_chunks(10, 4, 3, indexes, 10, 3).is_err());
    }
    assert!(plan_missing_chunks(10, 4, 3, &[0], 10, 65_537).is_err());
    assert!(plan_missing_chunks(10, 0, 3, &[0], 10, 3).is_err());
}

#[test]
fn handles_the_largest_u64_range() {
    assert_eq!(
        plan_chunks(u64::MAX, u64::MAX, 1).unwrap(),
        vec![Chunk {
            offset: 0,
            length: u64::MAX
        }]
    );
}
#[test]
fn sparse_resume_uses_stable_indexes_at_maximum_chunk_count() {
    let selected = plan_missing_chunks(65_536, 1, 65_536, &[0, 32_768, 65_535], 2, 3).unwrap();
    assert_eq!(selected.len(), 2);
    assert_eq!(
        selected[0].chunk,
        Chunk {
            offset: 0,
            length: 1
        }
    );
    assert_eq!(
        selected[1],
        IndexedChunk {
            index: 32_768,
            chunk: Chunk {
                offset: 32_768,
                length: 1
            },
        }
    );
    assert!(plan_missing_chunks(65_537, 1, 65_536, &[], 0, 1).is_err());
}
