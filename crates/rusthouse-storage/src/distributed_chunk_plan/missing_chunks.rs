//! Keep missing-chunk scheduling and paging separate from basic chunk arithmetic.

use super::*;

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
