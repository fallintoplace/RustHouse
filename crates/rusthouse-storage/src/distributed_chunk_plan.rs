//! Deterministic byte ranges for resumable immutable-object copies.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub offset: u64,
    pub length: u64,
}

fn checked_chunk_count(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
) -> Result<usize, &'static str> {
    if chunk_bytes == 0 || max_chunks == 0 || max_chunks > 65_536 {
        return Err("invalid transfer chunk limits");
    }
    let count = total_bytes.div_ceil(chunk_bytes);
    if count > max_chunks as u64 {
        return Err("transfer exceeds chunk count limit");
    }
    Ok(count as usize)
}

fn chunk_at_index(total_bytes: u64, chunk_bytes: u64, index: usize) -> Chunk {
    let offset = index as u64 * chunk_bytes;
    Chunk {
        offset,
        length: (total_bytes - offset).min(chunk_bytes),
    }
}

/// Plan nonoverlapping ranges. Empty files have no data chunks.
pub fn plan_chunks(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
) -> Result<Vec<Chunk>, &'static str> {
    let count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    Ok((0..count)
        .map(|index| chunk_at_index(total_bytes, chunk_bytes, index))
        .collect())
}

/// A pending byte range plus its stable index in the complete transfer
/// plan. The index can be persisted in a ResumeMap after verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedChunk {
    pub index: usize,
    pub chunk: Chunk,
}

fn indexed_chunk_at(total_bytes: u64, chunk_bytes: u64, index: usize) -> IndexedChunk {
    IndexedChunk {
        index,
        chunk: chunk_at_index(total_bytes, chunk_bytes, index),
    }
}

/// One contiguous remote I/O request covering adjacent verified chunk slots.
/// The caller must verify each constituent chunk before marking it complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkRun {
    pub first_index: usize,
    pub chunk_count: usize,
    pub offset: u64,
    pub length: u64,
}

/// Combine adjacent transfer chunks into larger read/write requests without
/// changing the exact verified chunk layout or its original ordering.
///
/// The input may contain gaps for already completed checkpoint chunks.
/// Every span is validated before returning any part of the coalesced plan.
pub fn coalesce_indexed_chunks(
    chunks: &[IndexedChunk],
    max_run_bytes: u64,
) -> Result<Vec<ChunkRun>, &'static str> {
    if max_run_bytes == 0 || chunks.len() > 65_536 {
        return Err("invalid coalesced chunk limits");
    }
    let mut previous_end = 0u64;
    let mut previous_index = None;
    let mut runs: Vec<ChunkRun> = Vec::new();
    for chunk in chunks {
        let end = chunk.chunk.offset.checked_add(chunk.chunk.length)
            .ok_or("transfer chunk range overflow")?;
        if chunk.chunk.length == 0 || chunk.chunk.length > max_run_bytes
            || chunk.chunk.offset < previous_end
            || previous_index.is_some_and(|index| chunk.index <= index)
        {
            return Err("invalid or overlapping transfer chunk");
        }
        previous_end = end;
        previous_index = Some(chunk.index);
        let item = chunk.chunk;
        if let Some(last) = runs.last_mut() {
            let next_index = last.first_index + last.chunk_count;
            let end = last.offset + last.length;
            if chunk.index == next_index && item.offset == end
                && item.length <= max_run_bytes - last.length
            {
                last.chunk_count += 1;
                last.length += item.length;
                continue;
            }
        }
        runs.push(ChunkRun {
            first_index: chunk.index,
            chunk_count: 1,
            offset: item.offset,
            length: item.length,
        });
    }
    Ok(runs)
}

#[cfg(test)]
mod coalesced_chunk_runs_tests {
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
                        bytes, 7, 65_536, &checkpoint, bytes, 65_536,
                    ).unwrap();
                    let runs = coalesce_indexed_chunks(&planned, run_budget).unwrap();
                    let expanded = runs.iter().flat_map(|run| {
                        (0..run.chunk_count).map(move |shift| {
                            let index = run.first_index + shift;
                            indexed_chunk_at(bytes, 7, index)
                        })
                    }).collect::<Vec<_>>();
                    assert_eq!(expanded, planned, "bytes={bytes}, stride={stride}, run_budget={run_budget}");
                    assert!(runs.iter().all(|run| run.length <= run_budget));
                    assert_eq!(runs.iter().map(|run| run.length).sum::<u64>(),
                        planned.iter().map(|item| item.chunk.length).sum::<u64>());
                    assert!(runs.windows(2).all(|pair| pair[0].first_index < pair[1].first_index));
                }
            }
        }
    }

    #[test]
    fn fully_missing_chunks_coalesce_to_minimum_io_requests() {
        let plan = plan_chunks(10_000, 1000, 10).unwrap();
        let indexed = plan.into_iter().enumerate()
            .map(|(index, chunk)| IndexedChunk { index, chunk }).collect::<Vec<_>>();
        let runs = coalesce_indexed_chunks(&indexed, 4000).unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0], ChunkRun {
            first_index: 0, chunk_count: 4, offset: 0, length: 4000,
        });
        assert_eq!(runs[1].chunk_count, 4);
        assert_eq!(runs[2].chunk_count, 2);
        assert_eq!(coalesce_indexed_chunks(&indexed, 1000).unwrap().len(), 10);
    }

    #[test]
    fn invalid_ranges_and_budget_fail_before_returning_partial_plan() {
        let sample = IndexedChunk { index: 0, chunk: Chunk { offset: 0, length: 7 } };
        assert!(coalesce_indexed_chunks(&[sample], 0).is_err());
        assert!(coalesce_indexed_chunks(&[sample], 6).is_err());
        assert!(coalesce_indexed_chunks(&[
            sample, IndexedChunk { index: 1, chunk: Chunk { offset: 6, length: 1 } },
        ], 8).is_err());
        assert!(coalesce_indexed_chunks(&[
            sample, IndexedChunk { index: 0, chunk: Chunk { offset: 7, length: 1 } },
        ], 8).is_err());
        assert!(coalesce_indexed_chunks(&[
            IndexedChunk { index: 2, chunk: Chunk { offset: u64::MAX, length: 2 } },
        ], u64::MAX).is_err());
        assert_eq!(coalesce_indexed_chunks(&[], 1024).unwrap(), Vec::<ChunkRun>::new());
    }
}

#[cfg(test)]
mod fused_coalescing_regression_tests {
    use super::*;

    #[test]
    fn rejects_late_invalid_chunk_even_after_merging_earlier_runs() {
        let chunks = [
            IndexedChunk { index: 0, chunk: Chunk { offset: 0, length: 8 } },
            IndexedChunk { index: 1, chunk: Chunk { offset: 8, length: 8 } },
            IndexedChunk { index: 2, chunk: Chunk { offset: 16, length: 8 } },
            IndexedChunk { index: 3, chunk: Chunk { offset: 23, length: 1 } },
        ];
        assert!(coalesce_indexed_chunks(&chunks, 32).is_err());
    }

    #[test]
    fn large_gapped_plan_preserves_run_budgets_and_boundaries() {
        let chunks = (0..4096usize).filter(|index| index % 17 != 0)
            .map(|index| IndexedChunk {
                index,
                chunk: Chunk { offset: index as u64 * 1024, length: 1024 },
            }).collect::<Vec<_>>();
        let runs = coalesce_indexed_chunks(&chunks, 8 * 1024).unwrap();
        assert!(runs.iter().all(|run| run.length <= 8 * 1024));
        let expanded = runs.iter().flat_map(|run| {
            (0..run.chunk_count).map(move |offset| run.first_index + offset)
        }).collect::<Vec<_>>();
        assert_eq!(expanded, chunks.iter().map(|chunk| chunk.index).collect::<Vec<_>>());
        assert_eq!(runs.iter().map(|run| run.length).sum::<u64>(),
            chunks.iter().map(|chunk| chunk.chunk.length).sum::<u64>());
    }
}

/// Select an ordered, bounded prefix from ResumeMap::remaining().
///
/// Validate strictly increasing indexes so a corrupt progress checkpoint
/// cannot duplicate writes or silently omit ranges. No network work occurs.
pub fn plan_missing_chunks(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    remaining_indices: &[usize],
    byte_budget: u64,
    max_batch_chunks: usize,
) -> Result<Vec<IndexedChunk>, &'static str> {
    if max_batch_chunks > 65_536 {
        return Err("transfer batch chunk limit exceeds bound");
    }
    let chunk_count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    let mut previous_chunk_index = None;
    for &index in remaining_indices {
        if index >= chunk_count || previous_chunk_index.is_some_and(|last| index <= last) {
            return Err("invalid or duplicate missing chunk index");
        }
        previous_chunk_index = Some(index);
    }
    // Validate every missing index first, even when no work can be scheduled.
    if byte_budget == 0 || max_batch_chunks == 0 || remaining_indices.is_empty() {
        return Ok(Vec::new());
    }
    // Only full chunks plus possibly one short final chunk can fit this budget.
    let budget_chunks = byte_budget / chunk_bytes;
    let reserve = remaining_indices
        .len()
        .min(max_batch_chunks)
        .min(usize::try_from(budget_chunks.saturating_add(1)).unwrap_or(usize::MAX));
    let mut selected = Vec::with_capacity(reserve);
    let mut remaining_bytes = byte_budget;
    for &index in remaining_indices {
        if selected.len() >= max_batch_chunks || remaining_bytes == 0 {
            break;
        }
        let indexed = indexed_chunk_at(total_bytes, chunk_bytes, index);
        if indexed.chunk.length > remaining_bytes {
            continue;
        }
        remaining_bytes -= indexed.chunk.length;
        selected.push(indexed);
    }
    Ok(selected)
}


/// Schedule only incomplete chunks directly from a bit-packed checkpoint.
///
/// Unlike building ResumeMap::remaining() first, this keeps temporary
/// memory proportional to the selected batch, not the full transfer.
pub fn plan_missing_chunks_from_resume(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    progress: &crate::distributed_resume_map::ResumeMap,
    byte_budget: u64,
    max_batch_chunks: usize,
) -> Result<Vec<IndexedChunk>, &'static str> {
    if max_batch_chunks > 65_536 {
        return Err("transfer batch chunk limit exceeds bound");
    }
    let chunk_count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    if progress.len() != chunk_count {
        return Err("resume checkpoint does not match transfer layout");
    }
    if byte_budget == 0 || max_batch_chunks == 0 {
        return Ok(Vec::new());
    }
    let max_by_budget = byte_budget / chunk_bytes;
    let reserve = progress.remaining_count()
        .min(max_batch_chunks)
        .min(usize::try_from(max_by_budget.saturating_add(1)).unwrap_or(usize::MAX));
    let mut selected = Vec::with_capacity(reserve);
    let mut remaining_bytes = byte_budget;
    for index in progress.remaining_indices() {
        if selected.len() == max_batch_chunks || remaining_bytes == 0 {
            break;
        }
        let indexed = indexed_chunk_at(total_bytes, chunk_bytes, index);
        if indexed.chunk.length <= remaining_bytes {
            remaining_bytes -= indexed.chunk.length;
            selected.push(indexed);
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod checkpoint_scheduling_tests {
    use super::*;
    use crate::distributed_resume_map::ResumeMap;

    #[test]
    fn agrees_with_indexed_planner_across_sparse_and_dense_checkpoints() {
        for bytes in [0u64, 1, 7, 8, 9, 65, 4097, 65_536] {
            let count = bytes.div_ceil(7) as usize;
            for complete_modulus in [1, 2, 7, 31] {
                let mut map = ResumeMap::new(count).unwrap();
                for index in (0..count).step_by(complete_modulus) {
                    map.mark_complete(index).unwrap();
                }
                for budget in [0, 1, 2, 6, 7, 13, 14, 21, 1000] {
                    for limit in [0, 1, 2, 5, 128] {
                        let expected = plan_missing_chunks(bytes, 7, 65_536, &map.remaining(), budget, limit).unwrap();
                        let actual = plan_missing_chunks_from_resume(bytes, 7, 65_536, &map, budget, limit).unwrap();
                        assert_eq!(actual, expected, "{bytes} {complete_modulus} {budget} {limit}");
                    }
                }
            }
        }
    }

    #[test]
    fn transfer_bounds_and_mismatched_checkpoints_fail_closed() {
        let map = ResumeMap::new(2).unwrap();
        assert!(plan_missing_chunks_from_resume(10, 4, 3, &map, 0, 0).is_err());
        assert!(plan_missing_chunks_from_resume(10, 0, 3, &map, 10, 3).is_err());
        assert!(plan_missing_chunks_from_resume(8, 4, 3, &map, 8, 65_537).is_err());
        let huge = ResumeMap::new(65_536).unwrap();
        let selected = plan_missing_chunks_from_resume(65_536, 1, 65_536, &huge, 3, 3).unwrap();
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[2].index, 2);
    }
}

fn build_chunk_window(
    total_bytes: u64,
    chunk_bytes: u64,
    start_index: usize,
    requested: usize,
) -> Vec<IndexedChunk> {
    let mut page = Vec::with_capacity(requested);
    for index in start_index..start_index + requested {
        page.push(indexed_chunk_at(total_bytes, chunk_bytes, index));
    }
    page
}

/// Page through a transfer plan without allocating the entire chunk list.
pub fn plan_chunk_window(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    start_index: usize,
    requested: usize,
) -> Result<Vec<IndexedChunk>, &'static str> {
    let count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    if start_index > count || requested > count - start_index {
        return Err("transfer chunk window exceeds bounds");
    }
    Ok(build_chunk_window(total_bytes, chunk_bytes, start_index, requested))
}

/// Return up to requested chunks, stopping at the end of the transfer.
///
/// The strict plan_chunk_window keeps its original out-of-bounds semantics.
/// This variant is useful for callers that page through a transfer with a
/// fixed page size and cannot predict the final partial page.
pub fn plan_chunk_window_clamped(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    start_index: usize,
    requested: usize,
) -> Result<Vec<IndexedChunk>, &'static str> {
    let count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    if start_index > count {
        return Err("transfer chunk window exceeds bounds");
    }
    Ok(build_chunk_window(
        total_bytes,
        chunk_bytes,
        start_index,
        requested.min(count - start_index),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamped_windows_return_partial_final_pages() {
        let all = plan_chunks(10, 3, 4).unwrap();
        let first = plan_chunk_window_clamped(10, 3, 4, 0, 3).unwrap();
        let tail = plan_chunk_window_clamped(10, 3, 4, 3, 3).unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].chunk, all[3]);
        assert!(plan_chunk_window_clamped(10, 3, 4, 4, usize::MAX)
            .unwrap()
            .is_empty());
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
                IndexedChunk { index: 0, chunk: Chunk { offset: 0, length: 4 } },
                IndexedChunk { index: 2, chunk: Chunk { offset: 8, length: 2 } },
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
        assert!(plan_missing_chunks(65_536, 1, 65_536, &indexes, 0, 65_536)
            .unwrap()
            .is_empty());
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
}

#[cfg(test)]
mod chunk_window_tests {
    use super::*;

    #[test]
    fn pages_partial_tail_with_original_indexes() {
        assert_eq!(
            plan_chunk_window(10, 4, 3, 1, 2).unwrap(),
            vec![
                IndexedChunk {
                    index: 1,
                    chunk: Chunk { offset: 4, length: 4 },
                },
                IndexedChunk {
                    index: 2,
                    chunk: Chunk { offset: 8, length: 2 },
                },
            ]
        );
        assert!(plan_chunk_window(10, 4, 3, 2, 2).is_err());
        assert!(plan_chunk_window(10, 4, 3, 4, 0).is_err());
        assert!(plan_chunk_window(0, 4, 3, 0, 0).unwrap().is_empty());
    }

    #[test]
    fn large_transfers_allocate_only_requested_chunks() {
        let page = plan_chunk_window(65_536, 1, 65_536, 65_534, 2).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[1].index, 65_535);
    }
}

#[cfg(test)]
mod skipped_chunk_budget_tests {
    use super::*;

    #[test]
    fn smaller_tail_can_progress_when_first_chunk_does_not_fit() {
        assert_eq!(
            plan_missing_chunks(10, 4, 3, &[0, 2], 2, 2).unwrap(),
            vec![IndexedChunk { index: 2, chunk: Chunk { offset: 8, length: 2 } }]
        );
        assert!(plan_missing_chunks(10, 4, 3, &[0, 2], 1, 2).unwrap().is_empty());
        assert_eq!(plan_missing_chunks(10, 4, 3, &[0, 1, 2], 6, 3).unwrap().len(), 2);
    }
}

#[cfg(test)]
mod clamped_window_fast_path_tests {
    use super::*;

    #[test]
    fn strict_and_clamped_windows_match_within_bounds() {
        for total in [0_u64, 1, 7, 8, 9, 33, 127, 256, 1025] {
            let count = total.div_ceil(7);
            for start in 0..=count {
                for request in 0..=count - start {
                    let strict = plan_chunk_window(total, 7, 65536, start as usize, request as usize).unwrap();
                    let clamped = plan_chunk_window_clamped(total, 7, 65536, start as usize, request as usize).unwrap();
                    assert_eq!(strict, clamped);
                }
            }
        }
    }
}
