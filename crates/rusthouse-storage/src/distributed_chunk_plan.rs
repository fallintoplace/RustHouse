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
        if chunk.index >= 65_536
            || item.length == 0
            || item.length > max_run_bytes
            || item.offset < previous_end
            || previous_index.is_some_and(|index| chunk.index <= index)
        {
            return Err("invalid or overlapping transfer chunk");
        }
        previous_end = end;
        previous_index = Some(chunk.index);
        if let Some(last) = runs.last_mut() {
            let next_index = last
                .first_index
                .checked_add(last.chunk_count)
                .ok_or("transfer run index overflow")?;
            let run_end = last
                .offset
                .checked_add(last.length)
                .ok_or("transfer run byte range overflow")?;
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

/// Validate sparse indexed chunks against their canonical transfer layout
/// before combining them into network I/O runs.
///
/// The ordinary coalescer checks ordering and overlap. This stricter entry
/// point also proves that each index refers to its exact planned byte range,
/// including the shorter final chunk, and rejects forged resume metadata.
pub fn coalesce_canonical_indexed_chunks(
    chunks: &[IndexedChunk],
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    max_run_bytes: u64,
) -> Result<Vec<ChunkRun>, &'static str> {
    let count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    if chunks.len() > count {
        return Err("too many indexed transfer chunks");
    }
    for chunk in chunks {
        if chunk.index >= count
            || chunk.chunk != chunk_at_index(total_bytes, chunk_bytes, chunk.index)
        {
            return Err("indexed chunk differs from canonical transfer layout");
        }
    }
    coalesce_indexed_chunks(chunks, max_run_bytes)
}

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/coalesced_chunk_runs_tests.rs"]
mod coalesced_chunk_runs_tests;

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
#[path = "distributed_chunk_plan_tests/direct_resumable_run_tests.rs"]
mod direct_resumable_run_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/checkpoint_scheduling_tests.rs"]
mod checkpoint_scheduling_tests;

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
#[path = "distributed_chunk_plan_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/chunk_window_tests.rs"]
mod chunk_window_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/skipped_chunk_budget_tests.rs"]
mod skipped_chunk_budget_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/clamped_window_fast_path_tests.rs"]
mod clamped_window_fast_path_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/tiny_checkpoint_page_tests.rs"]
mod tiny_checkpoint_page_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/short_tail_binary_search_tests.rs"]
mod short_tail_binary_search_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/single_pass_coalescing_regression_tests.rs"]
mod single_pass_coalescing_regression_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/paged_resume_planner_tests.rs"]
mod paged_resume_planner_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/paged_coalesced_resume_reference_tests.rs"]
mod paged_coalesced_resume_reference_tests;

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
    // A full chunk that already exceeds the entire byte budget cannot
    // become affordable as the batch fills. Only its shorter tail might
    // fit. Jump straight to that final slot instead of scanning up to
    // 65,535 preceding, untransferable missing chunks per object.
    let mut iterators = sources
        .iter()
        .map(|source| {
            let start = if source.chunk_bytes > byte_budget {
                source.checkpoint.len().saturating_sub(1)
            } else {
                0
            };
            source.checkpoint.remaining_indices_from(start)
        })
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
#[path = "distributed_chunk_plan_tests/fair_replica_transfer_tests.rs"]
mod fair_replica_transfer_tests;

/// Map a half-open changed byte span to every transfer chunk it touches.
///
/// The returned range contains chunk indexes, not byte offsets. An empty
/// byte span never invalidates an already verified chunk, even when it is
/// positioned exactly at EOF. The caller must still verify object identity.
pub fn chunks_intersecting_range(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    offset: u64,
    length: u64,
) -> Result<std::ops::Range<usize>, &'static str> {
    let count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    let end = offset
        .checked_add(length)
        .ok_or("changed object range overflow")?;
    if offset > total_bytes || end > total_bytes {
        return Err("changed object range exceeds transfer layout");
    }
    if length == 0 {
        let cursor = (offset / chunk_bytes).min(count as u64) as usize;
        return Ok(cursor..cursor);
    }
    let first = (offset / chunk_bytes) as usize;
    let last = ((end - 1) / chunk_bytes + 1) as usize;
    Ok(first..last)
}

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/intersecting_chunk_range_tests.rs"]
mod intersecting_chunk_range_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/lossless_missing_chunk_page_tests.rs"]
mod lossless_missing_chunk_page_tests;

#[cfg(test)]
#[path = "distributed_chunk_plan_tests/fair_transfer_tiny_budget_skip_tests.rs"]
mod fair_transfer_tiny_budget_skip_tests;
