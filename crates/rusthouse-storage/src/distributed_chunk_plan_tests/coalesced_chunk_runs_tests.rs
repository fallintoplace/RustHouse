use super::*;
use crate::distributed_resume_map::ResumeMap;

#[test]
fn complete_and_sparse_transfers_keep_exact_chunk_coverage() {
    for bytes in [1u64, 7, 8, 9, 100, 4097, 65_535] {
        let total = bytes.div_ceil(7) as usize;
        for stride in [2usize, 3, 7, 23] {
            let mut checkpoint = ResumeMap::new(total).unwrap();
            for index in (0..total).step_by(stride) {
                checkpoint.mark_complete(index).unwrap();
            }
            for run_budget in [7u64, 14, 28, 56, 4096] {
                let planned = plan_missing_chunks_from_resume(
                    bytes,
                    7,
                    65_536,
                    &checkpoint,
                    bytes,
                    65_536,
                )
                .unwrap();
                let runs = coalesce_indexed_chunks(&planned, run_budget).unwrap();
                let expanded = runs
                    .iter()
                    .flat_map(|run| {
                        (0..run.chunk_count).map(move |shift| {
                            let index = run.first_index + shift;
                            indexed_chunk_at(bytes, 7, index)
                        })
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    expanded, planned,
                    "bytes={bytes}, stride={stride}, run_budget={run_budget}"
                );
                assert!(runs.iter().all(|run| run.length <= run_budget));
                assert_eq!(
                    runs.iter().map(|run| run.length).sum::<u64>(),
                    planned.iter().map(|item| item.chunk.length).sum::<u64>()
                );
                assert!(
                    runs.windows(2)
                        .all(|pair| pair[0].first_index < pair[1].first_index)
                );
            }
        }
    }
}

#[test]
fn canonical_coalescing_rejects_forged_resume_ranges() {
    let sparse = [
        IndexedChunk {
            index: 0,
            chunk: Chunk {
                offset: 0,
                length: 7,
            },
        },
        IndexedChunk {
            index: 2,
            chunk: Chunk {
                offset: 14,
                length: 6,
            },
        },
    ];
    let runs = coalesce_canonical_indexed_chunks(&sparse, 20, 7, 4, 20).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].offset, 0);
    assert_eq!(runs[1].offset, 14);

    let mut forged = sparse;
    forged[1].chunk.offset = 13;
    assert!(coalesce_canonical_indexed_chunks(&forged, 20, 7, 4, 20).is_err());
    forged = sparse;
    forged[1].chunk.length = 7;
    assert!(coalesce_canonical_indexed_chunks(&forged, 20, 7, 4, 20).is_err());
    forged = sparse;
    forged[1].index = 3;
    assert!(coalesce_canonical_indexed_chunks(&forged, 20, 7, 4, 20).is_err());
    assert!(coalesce_canonical_indexed_chunks(&sparse, 20, 0, 4, 20).is_err());
    assert!(coalesce_canonical_indexed_chunks(&sparse, 20, 7, 2, 20).is_err());
    assert!(
        coalesce_canonical_indexed_chunks(&[], 0, 7, 1, 20)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn fully_missing_chunks_coalesce_to_minimum_io_requests() {
    let plan = plan_chunks(10_000, 1000, 10).unwrap();
    let indexed = plan
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| IndexedChunk { index, chunk })
        .collect::<Vec<_>>();
    let runs = coalesce_indexed_chunks(&indexed, 4000).unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(
        runs[0],
        ChunkRun {
            first_index: 0,
            chunk_count: 4,
            offset: 0,
            length: 4000,
        }
    );
    assert_eq!(runs[1].chunk_count, 4);
    assert_eq!(runs[2].chunk_count, 2);
    assert_eq!(coalesce_indexed_chunks(&indexed, 1000).unwrap().len(), 10);
}

#[test]
fn coalesced_runs_reject_impossible_indexes_without_panicking() {
    for index in [65_536, usize::MAX] {
        let bad = IndexedChunk {
            index,
            chunk: Chunk {
                offset: 0,
                length: 1,
            },
        };
        assert!(coalesce_indexed_chunks(&[bad], 10).is_err());
        let valid = IndexedChunk {
            index: 65_535,
            chunk: Chunk {
                offset: 0,
                length: 1,
            },
        };
        assert!(coalesce_indexed_chunks(&[valid, bad], 10).is_err());
    }
    assert_eq!(
        coalesce_indexed_chunks(
            &[IndexedChunk {
                index: 65_535,
                chunk: Chunk {
                    offset: u64::MAX - 1,
                    length: 1
                },
            }],
            u64::MAX,
        )
        .unwrap()[0]
            .first_index,
        65_535,
    );
}

#[test]
fn invalid_ranges_and_budget_fail_before_returning_partial_plan() {
    let sample = IndexedChunk {
        index: 0,
        chunk: Chunk {
            offset: 0,
            length: 7,
        },
    };
    assert!(coalesce_indexed_chunks(&[sample], 0).is_err());
    assert!(coalesce_indexed_chunks(&[sample], 6).is_err());
    assert!(
        coalesce_indexed_chunks(
            &[
                sample,
                IndexedChunk {
                    index: 1,
                    chunk: Chunk {
                        offset: 6,
                        length: 1
                    }
                },
            ],
            8
        )
        .is_err()
    );
    assert!(
        coalesce_indexed_chunks(
            &[
                sample,
                IndexedChunk {
                    index: 0,
                    chunk: Chunk {
                        offset: 7,
                        length: 1
                    }
                },
            ],
            8
        )
        .is_err()
    );
    assert!(
        coalesce_indexed_chunks(
            &[IndexedChunk {
                index: 2,
                chunk: Chunk {
                    offset: u64::MAX,
                    length: 2
                }
            },],
            u64::MAX
        )
        .is_err()
    );
    assert_eq!(
        coalesce_indexed_chunks(&[], 1024).unwrap(),
        Vec::<ChunkRun>::new()
    );
}
