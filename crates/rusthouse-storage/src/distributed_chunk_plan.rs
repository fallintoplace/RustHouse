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
    // The output stays local until the entire input has been validated.
    // Validate and coalesce in one walk instead of traversing all chunks twice.
    for chunk in chunks {
        let item = chunk.chunk;
        let end = item
            .offset
            .checked_add(item.length)
            .ok_or("transfer chunk range overflow")?;
        if item.length == 0
            || item.length > max_run_bytes
            || item.offset < previous_end
            || previous_index.is_some_and(|index| chunk.index <= index)
        {
            return Err("invalid or overlapping transfer chunk");
        }
        previous_end = end;
        previous_index = Some(chunk.index);
        if let Some(last) = runs.last_mut() {
            let next_index = last.first_index + last.chunk_count;
            let run_end = last.offset + last.length;
            if chunk.index == next_index
                && item.offset == run_end
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
    // When a budget cannot fit a normal chunk, only the short final chunk
    // can qualify. The missing indices are validated and strictly sorted,
    // so avoid a full scan of a large resume inventory.
    if byte_budget < chunk_bytes {
        let final_index = chunk_count - 1;
        if remaining_indices.binary_search(&final_index).is_ok() {
            let final_chunk = indexed_chunk_at(total_bytes, chunk_bytes, final_index);
            if final_chunk.chunk.length <= byte_budget {
                return Ok(vec![final_chunk]);
            }
        }
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
    plan_missing_chunks_from_resume_after(
        total_bytes,
        chunk_bytes,
        max_chunks,
        progress,
        0,
        byte_budget,
        max_batch_chunks,
    )
}

/// Resume planning from an inclusive chunk cursor without scanning earlier
/// completion bits. Validation remains strict even for empty page budgets.
pub fn plan_missing_chunks_from_resume_after(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    progress: &crate::distributed_resume_map::ResumeMap,
    start_index: usize,
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
    if start_index > chunk_count {
        return Err("resume cursor exceeds transfer layout");
    }
    if byte_budget == 0 || max_batch_chunks == 0 {
        return Ok(Vec::new());
    }
    let max_by_budget = byte_budget / chunk_bytes;
    // Small pages need no whole-checkpoint popcount merely to size their
    // output. A one-chunk scheduler otherwise rescans all 65,536 bitmap
    // positions for every page, producing quadratic planning work.
    // For large pages, count remaining bits to avoid over-allocating when
    // a nearly complete transfer has only a few holes.
    let reserve_limit = (progress.len() - start_index)
        .min(max_batch_chunks)
        .min(usize::try_from(max_by_budget.saturating_add(1)).unwrap_or(usize::MAX));
    let reserve = if reserve_limit <= 64 {
        reserve_limit
    } else {
        reserve_limit.min(progress.remaining_indices_from(start_index).count())
    };
    let mut selected = Vec::with_capacity(reserve);
    let mut remaining_bytes = byte_budget;
    for index in progress.remaining_indices_from(start_index) {
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

/// A lossless bounded transfer page over an authenticated resume bitmap.
/// The continuation index is inclusive and never advances past a missing chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingChunkPage {
    pub chunks: Vec<IndexedChunk>,
    pub next_start_index: Option<usize>,
}

/// Page incomplete transfer chunks without silently skipping an earlier chunk
/// that cannot fit the remaining byte budget.
///
/// Existing greedy planners may skip unaffordable chunks to fill each batch.
/// This version preserves strict ordered continuity for consumers advancing
/// their resume cursor after each batch. A blocked first chunk is an error.
#[allow(clippy::too_many_arguments)]
pub fn plan_missing_chunk_page_from_resume(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    progress: &crate::distributed_resume_map::ResumeMap,
    start_index: usize,
    byte_budget: u64,
    max_batch_chunks: usize,
) -> Result<MissingChunkPage, &'static str> {
    if max_batch_chunks == 0 || max_batch_chunks > 65_536 {
        return Err("invalid lossless transfer page limit");
    }
    let count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    if progress.len() != count {
        return Err("resume checkpoint does not match transfer layout");
    }
    if start_index > count {
        return Err("resume cursor exceeds transfer layout");
    }
    let mut remaining = byte_budget;
    let mut chunks = Vec::new();
    let mut has_more = false;
    for index in progress.remaining_indices_from(start_index) {
        if chunks.len() == max_batch_chunks {
            has_more = true;
            break;
        }
        let chunk = indexed_chunk_at(total_bytes, chunk_bytes, index);
        if chunk.chunk.length > remaining {
            if chunks.is_empty() {
                return Err("next missing transfer chunk exceeds byte budget");
            }
            has_more = true;
            break;
        }
        remaining -= chunk.chunk.length;
        chunks.push(chunk);
    }
    let next_start_index = if has_more {
        Some(chunks.last().expect("nonempty incomplete page").index + 1)
    } else {
        None
    };
    Ok(MissingChunkPage {
        chunks,
        next_start_index,
    })
}

/// Schedule coalesced transfer I/O directly from a packed completion map.
///
/// Unlike planning individual missing chunks and merging them afterwards,
/// memory is proportional to the output run count, even for dense transfers.
pub fn plan_missing_runs_from_resume(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    progress: &crate::distributed_resume_map::ResumeMap,
    byte_budget: u64,
    max_batch_chunks: usize,
    max_run_bytes: u64,
) -> Result<Vec<ChunkRun>, &'static str> {
    plan_missing_runs_from_resume_after(
        total_bytes,
        chunk_bytes,
        max_chunks,
        progress,
        0,
        byte_budget,
        max_batch_chunks,
        max_run_bytes,
    )
}

/// Resume coalesced I/O planning at an inclusive chunk cursor without
/// rescanning earlier completed checkpoint bits or materializing all chunks.
/// Validate the complete checkpoint layout before selecting any work.
#[allow(clippy::too_many_arguments)]
pub fn plan_missing_runs_from_resume_after(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    progress: &crate::distributed_resume_map::ResumeMap,
    start_index: usize,
    byte_budget: u64,
    max_batch_chunks: usize,
    max_run_bytes: u64,
) -> Result<Vec<ChunkRun>, &'static str> {
    if max_run_bytes == 0 || max_batch_chunks > 65_536 {
        return Err("invalid coalesced chunk limits");
    }
    let chunk_count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    if progress.len() != chunk_count {
        return Err("resume checkpoint does not match transfer layout");
    }
    if start_index > chunk_count {
        return Err("resume cursor exceeds transfer layout");
    }
    let mut runs: Vec<ChunkRun> = Vec::new();
    let mut scheduled = 0usize;
    let mut remaining = byte_budget;
    for index in progress.remaining_indices_from(start_index) {
        if scheduled == max_batch_chunks || remaining == 0 {
            break;
        }
        let chunk = chunk_at_index(total_bytes, chunk_bytes, index);
        if chunk.length > remaining {
            continue;
        }
        if chunk.length > max_run_bytes {
            return Err("invalid or overlapping transfer chunk");
        }
        if let Some(last) = runs.last_mut() {
            if last.first_index + last.chunk_count == index
                && last.offset + last.length == chunk.offset
                && chunk.length <= max_run_bytes - last.length
            {
                last.chunk_count += 1;
                last.length += chunk.length;
                scheduled += 1;
                remaining -= chunk.length;
                continue;
            }
        }
        runs.push(ChunkRun {
            first_index: index,
            chunk_count: 1,
            offset: chunk.offset,
            length: chunk.length,
        });
        scheduled += 1;
        remaining -= chunk.length;
    }
    Ok(runs)
}

#[cfg(test)]
mod direct_resumable_run_tests {
    use super::*;

    #[test]
    fn direct_runs_match_materialize_then_coalesce_over_sparse_checkpoints() {
        for bytes in [0u64, 1, 7, 8, 9, 63, 65, 4097, 65_536] {
            let count = bytes.div_ceil(7) as usize;
            for stride in [1usize, 2, 3, 13, 31] {
                let mut progress = crate::distributed_resume_map::ResumeMap::new(count).unwrap();
                for index in (0..count).step_by(stride) {
                    progress.mark_complete(index).unwrap();
                }
                for budget in [0, 1, 6, 7, 13, 21, 1000] {
                    for limit in [0, 1, 2, 7, 128] {
                        for max_run in [1, 6, 7, 14, 63, 8192] {
                            let reference = plan_missing_chunks_from_resume(
                                bytes, 7, 65_536, &progress, budget, limit,
                            )
                            .and_then(|chunks| coalesce_indexed_chunks(&chunks, max_run));
                            let planned = plan_missing_runs_from_resume(
                                bytes, 7, 65_536, &progress, budget, limit, max_run,
                            );
                            assert_eq!(
                                planned, reference,
                                "{bytes}:{stride}:{budget}:{limit}:{max_run}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_invalid_budgets_and_layouts_without_partial_runs() {
        let map = crate::distributed_resume_map::ResumeMap::new(2).unwrap();
        assert!(plan_missing_runs_from_resume(10, 4, 3, &map, 8, 2, 8).is_err());
        assert!(plan_missing_runs_from_resume(8, 4, 3, &map, 8, 2, 0).is_err());
        assert!(plan_missing_runs_from_resume(8, 4, 3, &map, 8, 65_537, 8).is_err());
        assert_eq!(
            plan_missing_runs_from_resume(8, 4, 3, &map, 8, 2, 8).unwrap(),
            vec![ChunkRun {
                first_index: 0,
                chunk_count: 2,
                offset: 0,
                length: 8
            }]
        );
    }
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
                        let expected =
                            plan_missing_chunks(bytes, 7, 65_536, &map.remaining(), budget, limit)
                                .unwrap();
                        let actual =
                            plan_missing_chunks_from_resume(bytes, 7, 65_536, &map, budget, limit)
                                .unwrap();
                        assert_eq!(
                            actual, expected,
                            "{bytes} {complete_modulus} {budget} {limit}"
                        );
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
    Ok(build_chunk_window(
        total_bytes,
        chunk_bytes,
        start_index,
        requested,
    ))
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
                    chunk: Chunk {
                        offset: 4,
                        length: 4
                    },
                },
                IndexedChunk {
                    index: 2,
                    chunk: Chunk {
                        offset: 8,
                        length: 2
                    },
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
            vec![IndexedChunk {
                index: 2,
                chunk: Chunk {
                    offset: 8,
                    length: 2
                }
            }]
        );
        assert!(
            plan_missing_chunks(10, 4, 3, &[0, 2], 1, 2)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            plan_missing_chunks(10, 4, 3, &[0, 1, 2], 6, 3)
                .unwrap()
                .len(),
            2
        );
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
                    let strict =
                        plan_chunk_window(total, 7, 65536, start as usize, request as usize)
                            .unwrap();
                    let clamped = plan_chunk_window_clamped(
                        total,
                        7,
                        65536,
                        start as usize,
                        request as usize,
                    )
                    .unwrap();
                    assert_eq!(strict, clamped);
                }
            }
        }
    }
}

#[cfg(test)]
mod tiny_checkpoint_page_tests {
    use super::*;
    use crate::distributed_resume_map::ResumeMap;

    #[test]
    fn tiny_pages_keep_order_and_budget_for_dense_and_sparse_checkpoints() {
        let chunks = 65_536;
        let mut progress = ResumeMap::new(chunks).unwrap();
        for index in 0..chunks {
            if index != 17 && index != 33_333 && index != 65_535 {
                progress.mark_complete(index).unwrap();
            }
        }
        for page_limit in [1, 2, 3, 4, 8, 64] {
            for byte_budget in [1, 2, 4, 64, u64::MAX] {
                let expected = plan_missing_chunks(
                    chunks as u64,
                    1,
                    chunks,
                    &progress.remaining(),
                    byte_budget,
                    page_limit,
                )
                .unwrap();
                let actual = plan_missing_chunks_from_resume(
                    chunks as u64,
                    1,
                    chunks,
                    &progress,
                    byte_budget,
                    page_limit,
                )
                .unwrap();
                assert_eq!(
                    actual, expected,
                    "page_limit={page_limit} budget={byte_budget}"
                );
            }
        }
    }
}

#[cfg(test)]
mod short_tail_binary_search_tests {
    use super::*;

    #[test]
    fn tiny_budget_only_schedules_the_short_final_chunk() {
        let missing = (0..65_536).collect::<Vec<_>>();
        let total_bytes = 65_535 * 8 + 3;
        let selected = plan_missing_chunks(total_bytes, 8, 65_536, &missing, 3, 4).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].index, 65_535);
        assert_eq!(selected[0].chunk.length, 3);
        assert!(
            plan_missing_chunks(total_bytes, 8, 65_536, &missing, 2, 4)
                .unwrap()
                .is_empty()
        );
        assert!(
            plan_missing_chunks(total_bytes, 8, 65_536, &missing[..65_535], 3, 4)
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod single_pass_coalescing_regression_tests {
    use super::*;

    #[test]
    fn consecutive_runs_and_completed_gaps_keep_original_indices() {
        let chunks = [
            IndexedChunk {
                index: 0,
                chunk: Chunk {
                    offset: 0,
                    length: 4,
                },
            },
            IndexedChunk {
                index: 1,
                chunk: Chunk {
                    offset: 4,
                    length: 4,
                },
            },
            IndexedChunk {
                index: 4,
                chunk: Chunk {
                    offset: 16,
                    length: 4,
                },
            },
            IndexedChunk {
                index: 5,
                chunk: Chunk {
                    offset: 20,
                    length: 4,
                },
            },
        ];
        assert_eq!(
            coalesce_indexed_chunks(&chunks, 8).unwrap(),
            vec![
                ChunkRun {
                    first_index: 0,
                    chunk_count: 2,
                    offset: 0,
                    length: 8
                },
                ChunkRun {
                    first_index: 4,
                    chunk_count: 2,
                    offset: 16,
                    length: 8
                },
            ]
        );
    }

    #[test]
    fn late_invalid_chunk_rejects_whole_coalescing_plan() {
        let chunks = [
            IndexedChunk {
                index: 0,
                chunk: Chunk {
                    offset: 0,
                    length: 4,
                },
            },
            IndexedChunk {
                index: 1,
                chunk: Chunk {
                    offset: 4,
                    length: 4,
                },
            },
            IndexedChunk {
                index: 2,
                chunk: Chunk {
                    offset: u64::MAX,
                    length: 2,
                },
            },
        ];
        assert_eq!(
            coalesce_indexed_chunks(&chunks, 8),
            Err("transfer chunk range overflow")
        );
    }
}

#[cfg(test)]
mod paged_resume_planner_tests {
    use super::*;
    use crate::distributed_resume_map::ResumeMap;

    #[test]
    fn cursored_pages_match_original_planner_on_filtered_missing_indexes() {
        for bytes in [0u64, 1, 5, 8, 9, 63, 64, 4097] {
            let chunks = bytes.div_ceil(5) as usize;
            let mut map = ResumeMap::new(chunks).unwrap();
            for index in (0..chunks).step_by(3) {
                map.mark_complete(index).unwrap();
            }
            for start in [0, 1, 7, 8, chunks / 2, chunks] {
                if start > chunks {
                    continue;
                }
                let subset = map
                    .remaining()
                    .into_iter()
                    .filter(|&index| index >= start)
                    .collect::<Vec<_>>();
                for budget in [0, 1, 4, 5, 9, 100] {
                    for limit in [0, 1, 2, 16, 128] {
                        let expected =
                            plan_missing_chunks(bytes, 5, 65_536, &subset, budget, limit).unwrap();
                        let actual = plan_missing_chunks_from_resume_after(
                            bytes, 5, 65_536, &map, start, budget, limit,
                        )
                        .unwrap();
                        assert_eq!(actual, expected);
                    }
                }
            }
            assert!(
                plan_missing_chunks_from_resume_after(bytes, 5, 65_536, &map, chunks + 1, 0, 0)
                    .is_err()
            );
        }
    }
}

#[cfg(test)]
mod paged_coalesced_resume_reference_tests {
    use super::*;
    use crate::distributed_resume_map::ResumeMap;

    #[test]
    fn paged_run_planning_matches_materialize_then_coalesce() {
        for total_bytes in [0u64, 1, 7, 8, 9, 63, 64, 65, 127, 128, 129, 2049] {
            let count = total_bytes.div_ceil(7) as usize;
            let mut progress = ResumeMap::new(count).unwrap();
            for index in 0..count {
                if index % 5 == 0 || index % 13 == 0 {
                    progress.mark_complete(index).unwrap();
                }
            }
            for cursor in 0..=count {
                for budget in [0, 1, 6, 7, 10, 21, 300] {
                    for limit in [0, 1, 2, 5, 32] {
                        for max_run in [1, 7, 14, 100] {
                            let expected = plan_missing_chunks_from_resume_after(
                                total_bytes,
                                7,
                                65_536,
                                &progress,
                                cursor,
                                budget,
                                limit,
                            )
                            .and_then(|chunks| coalesce_indexed_chunks(&chunks, max_run));
                            let actual = plan_missing_runs_from_resume_after(
                                total_bytes,
                                7,
                                65_536,
                                &progress,
                                cursor,
                                budget,
                                limit,
                                max_run,
                            );
                            assert_eq!(
                                actual, expected,
                                "bytes={total_bytes} cursor={cursor} budget={budget} limit={limit} max_run={max_run}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_invalid_cursors_even_for_empty_or_zero_budget_pages() {
        let progress = ResumeMap::new(2).unwrap();
        assert!(plan_missing_runs_from_resume_after(8, 4, 2, &progress, 3, 0, 0, 8).is_err());
        assert!(plan_missing_runs_from_resume_after(8, 4, 2, &progress, 2, 0, 0, 0).is_err());
        assert!(plan_missing_runs_from_resume_after(10, 4, 3, &progress, 0, 0, 0, 8).is_err());
        assert!(
            plan_missing_runs_from_resume_after(8, 4, 2, &progress, 2, 8, 4, 8)
                .unwrap()
                .is_empty()
        );
    }
}

/// One validated immutable object and its verified transfer checkpoint.
pub struct FairTransferSource<'a> {
    pub key: &'a crate::distributed_object_key::ObjectKey,
    pub total_bytes: u64,
    pub chunk_bytes: u64,
    pub checkpoint: &'a crate::distributed_resume_map::ResumeMap,
}

/// Work is addressed by the original request position, never by a copied key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FairTransferChunk {
    pub source_index: usize,
    pub indexed: IndexedChunk,
}

/// Interleave missing chunks from up to 64 immutable objects under one budget.
///
/// Layouts and identities are checked even when no bytes can be scheduled.
/// Skipped oversized chunks cannot consume the budget or starve smaller
/// objects. Returned work never marks its checkpoint complete.
pub fn plan_fair_replica_transfers(
    sources: &[FairTransferSource<'_>],
    byte_budget: u64,
    max_batch_chunks: usize,
) -> Result<Vec<FairTransferChunk>, &'static str> {
    if sources.is_empty() || sources.len() > 64 || max_batch_chunks > 65_536 {
        return Err("invalid fair transfer batch limits");
    }
    for (index, source) in sources.iter().enumerate() {
        let chunks = checked_chunk_count(source.total_bytes, source.chunk_bytes, 65_536)?;
        if source.checkpoint.len() != chunks {
            return Err("transfer checkpoint does not match object layout");
        }
        if sources[..index]
            .iter()
            .any(|earlier| earlier.key == source.key)
        {
            return Err("duplicate immutable object transfer");
        }
    }
    if byte_budget == 0 || max_batch_chunks == 0 {
        return Ok(Vec::new());
    }
    // Nothing can fit when every full chunk is larger than the budget and
    // no object's final short chunk can fit either. In that case, avoid
    // scanning as many as 64 x 65536 checkpoint bits merely to return [].
    // A short final chunk must still be considered even when full chunks do
    // not fit, so checking chunk_bytes alone would be incorrect.
    if sources.iter().all(|source| {
        if source.chunk_bytes <= byte_budget {
            return false;
        }
        let tail = source.total_bytes % source.chunk_bytes;
        tail == 0 || tail > byte_budget
    }) {
        return Ok(Vec::new());
    }
    let mut iterators = sources
        .iter()
        .map(|source| source.checkpoint.remaining_indices())
        .collect::<Vec<_>>();
    let mut remaining = byte_budget;
    let mut output = Vec::with_capacity(max_batch_chunks.min(sources.len() * 4));
    loop {
        let mut progress = false;
        for (source_index, source) in sources.iter().enumerate() {
            // A large chunk may be unaffordable while a shorter final chunk
            // remains schedulable. Consume its cursor rather than retrying
            // the same unaffordable work on every subsequent fair round.
            for index in iterators[source_index].by_ref() {
                let indexed = indexed_chunk_at(source.total_bytes, source.chunk_bytes, index);
                if indexed.chunk.length <= remaining {
                    remaining -= indexed.chunk.length;
                    output.push(FairTransferChunk {
                        source_index,
                        indexed,
                    });
                    progress = true;
                    break;
                }
            }
            if output.len() == max_batch_chunks || remaining == 0 {
                return Ok(output);
            }
        }
        if !progress {
            return Ok(output);
        }
    }
}

#[cfg(test)]
mod fair_replica_transfer_tests {
    use super::*;
    use crate::distributed_object_key::ObjectKey;
    use crate::distributed_resume_map::ResumeMap;

    fn key(name: &str) -> ObjectKey {
        ObjectKey::new("cluster", "shard", 1, "part", name).unwrap()
    }

    #[test]
    fn alternates_objects_and_respects_the_shared_byte_budget() {
        let keys = [key("a.bin"), key("b.bin")];
        let first = ResumeMap::new(6).unwrap();
        let mut second = ResumeMap::new(6).unwrap();
        second.mark_complete_indices(&[0, 2]).unwrap();
        let sources = [
            FairTransferSource {
                key: &keys[0],
                total_bytes: 6,
                chunk_bytes: 1,
                checkpoint: &first,
            },
            FairTransferSource {
                key: &keys[1],
                total_bytes: 6,
                chunk_bytes: 1,
                checkpoint: &second,
            },
        ];
        let selected = plan_fair_replica_transfers(&sources, 6, 6).unwrap();
        let pairs = selected
            .iter()
            .map(|item| (item.source_index, item.indexed.index))
            .collect::<Vec<_>>();
        assert_eq!(pairs, [(0, 0), (1, 1), (0, 1), (1, 3), (0, 2), (1, 4)]);
        assert_eq!(
            selected
                .iter()
                .map(|item| item.indexed.chunk.length)
                .sum::<u64>(),
            6
        );
        assert_eq!(first.completed_count(), 0);
        assert_eq!(second.completed_count(), 2);
    }

    #[test]
    fn short_tail_is_not_starved_by_full_unaffordable_chunks() {
        let keys = [key("large.bin"), key("tail.bin")];
        let full = ResumeMap::new(3).unwrap();
        let tail = ResumeMap::new(3).unwrap();
        let sources = [
            FairTransferSource {
                key: &keys[0],
                total_bytes: 24,
                chunk_bytes: 8,
                checkpoint: &full,
            },
            FairTransferSource {
                key: &keys[1],
                total_bytes: 17,
                chunk_bytes: 8,
                checkpoint: &tail,
            },
        ];
        assert_eq!(
            plan_fair_replica_transfers(&sources, 1, 8).unwrap(),
            [FairTransferChunk {
                source_index: 1,
                indexed: IndexedChunk {
                    index: 2,
                    chunk: Chunk {
                        offset: 16,
                        length: 1
                    }
                },
            }],
        );
    }

    #[test]
    fn small_budget_fast_rejects_large_complete_chunks_but_keeps_short_tails() {
        let keys = (0..64)
            .map(|index| key(&format!("file{index}.bin")))
            .collect::<Vec<_>>();
        let checkpoints = (0..64)
            .map(|_| ResumeMap::new(65_536).unwrap())
            .collect::<Vec<_>>();
        let sources = keys
            .iter()
            .zip(&checkpoints)
            .map(|(key, checkpoint)| FairTransferSource {
                key,
                total_bytes: 65_536 * 8,
                chunk_bytes: 8,
                checkpoint,
            })
            .collect::<Vec<_>>();
        assert!(plan_fair_replica_transfers(&sources, 1, 64).unwrap().is_empty());

        let short_tail = ResumeMap::new(2).unwrap();
        let tail_key = key("short.bin");
        let only_tail = [FairTransferSource {
            key: &tail_key,
            total_bytes: 9,
            chunk_bytes: 8,
            checkpoint: &short_tail,
        }];
        assert_eq!(
            plan_fair_replica_transfers(&only_tail, 1, 2).unwrap()[0].indexed.index,
            1
        );
    }

    #[test]
    fn rejects_bad_layouts_and_duplicate_objects_even_with_zero_budget() {
        let id = key("one.bin");
        let valid = ResumeMap::new(2).unwrap();
        let bad = ResumeMap::new(1).unwrap();
        let source = FairTransferSource {
            key: &id,
            total_bytes: 8,
            chunk_bytes: 4,
            checkpoint: &valid,
        };
        let duplicate = [
            FairTransferSource { ..source },
            FairTransferSource { ..source },
        ];
        assert!(plan_fair_replica_transfers(&duplicate, 0, 0).is_err());
        assert!(
            plan_fair_replica_transfers(
                &[FairTransferSource {
                    checkpoint: &bad,
                    ..source
                },],
                0,
                0
            )
            .is_err()
        );
        assert!(plan_fair_replica_transfers(&[source], 1, 65_537).is_err());
    }
}

#[cfg(test)]
mod lossless_missing_chunk_page_tests {
    use super::*;
    use crate::distributed_resume_map::ResumeMap;

    #[test]
    fn every_missing_chunk_survives_sequential_byte_limited_pages() {
        let map = ResumeMap::new(5).unwrap();
        let mut start = 0usize;
        let mut planned = Vec::new();
        for _ in 0..6 {
            let page = plan_missing_chunk_page_from_resume(
                29, 7, 65_536, &map, start, 8, 3,
            ).unwrap();
            planned.extend(page.chunks.iter().map(|chunk| chunk.index));
            match page.next_start_index {
                Some(next) => {
                    assert!(next > start);
                    start = next;
                }
                None => break,
            }
        }
        assert_eq!(planned, [0, 1, 2, 3, 4]);
        let mut completed = map;
        completed.mark_complete_range(0, 3).unwrap();
        let page = plan_missing_chunk_page_from_resume(
            29, 7, 65_536, &completed, 0, 8, 3,
        ).unwrap();
        assert_eq!(
            page.chunks.iter().map(|chunk| chunk.index).collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert!(page.next_start_index.is_none());
    }

    #[test]
    fn unaffordable_first_chunk_cannot_advance_past_it() {
        let map = ResumeMap::new(3).unwrap();
        assert!(plan_missing_chunk_page_from_resume(
            15, 7, 65_536, &map, 0, 6, 9,
        ).is_err());
        let final_page = plan_missing_chunk_page_from_resume(
            15, 7, 65_536, &map, 2, 6, 9,
        ).unwrap();
        assert_eq!(final_page.chunks[0].index, 2);
        assert!(final_page.next_start_index.is_none());
    }

    #[test]
    fn rejects_mismatched_checkpoints_and_invalid_cursors() {
        let empty = ResumeMap::new(0).unwrap();
        let page = plan_missing_chunk_page_from_resume(
            0, 7, 65_536, &empty, 0, 0, 1,
        ).unwrap();
        assert!(page.chunks.is_empty());
        assert!(page.next_start_index.is_none());
        assert!(plan_missing_chunk_page_from_resume(
            8, 7, 65_536, &empty, 0, 7, 1,
        ).is_err());
        assert!(plan_missing_chunk_page_from_resume(
            0, 7, 65_536, &empty, 1, 7, 1,
        ).is_err());
        assert!(plan_missing_chunk_page_from_resume(
            0, 7, 65_536, &empty, 0, 7, 0,
        ).is_err());
    }
}
