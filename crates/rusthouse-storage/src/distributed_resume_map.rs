//! Bounded completion map for retrying interrupted chunk transfers.
mod checkpoints;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeMap {
    // Bit-packed completion flags: 8 times smaller than Vec<bool> when every
    // Rust bool occupies a byte, with the original ASCII encoding retained.
    chunks: usize,
    completed: Vec<u8>,
}

fn low_bit_mask(bits: usize) -> u8 {
    ((1u16 << bits) - 1) as u8
}

// Expand one packed checkpoint byte into its eight ASCII flags. A dense
// checkpoint is common near completion; 8-byte table copies avoid walking
// every individual set bit during serialization.
const ASCII_BITMAP_BYTES: [[u8; 8]; 256] = {
    let mut table = [[b'0'; 8]; 256];
    let mut byte = 0usize;
    while byte < 256 {
        let mut bit = 0usize;
        while bit < 8 {
            if byte & (1 << bit) != 0 {
                table[byte][bit] = b'1';
            }
            bit += 1;
        }
        byte += 1;
    }
    table
};

// Count completion flags a machine word at a time, rather than emitting
// one popcount for every byte in large resumable transfer checkpoints.
fn popcount_full_bytes(bytes: &[u8]) -> usize {
    let mut words = bytes.chunks_exact(8);
    let count = words
        .by_ref()
        .map(|chunk| {
            u64::from_ne_bytes(chunk.try_into().expect("eight-byte chunk")).count_ones() as usize
        })
        .sum::<usize>();
    count
        + words
            .remainder()
            .iter()
            .map(|byte| byte.count_ones() as usize)
            .sum::<usize>()
}

fn all_full_bytes_set(bytes: &[u8]) -> bool {
    let mut words = bytes.chunks_exact(8);
    words
        .by_ref()
        .all(|chunk| u64::from_ne_bytes(chunk.try_into().expect("eight-byte chunk")) == u64::MAX)
        && words.remainder().iter().all(|&byte| byte == u8::MAX)
}

impl ResumeMap {
    pub fn new(chunks: usize) -> Result<Self, &'static str> {
        if chunks > 65_536 {
            return Err("resume map exceeds chunk limit");
        }
        Ok(Self {
            chunks,
            completed: vec![0; chunks.div_ceil(8)],
        })
    }

    pub fn len(&self) -> usize {
        self.chunks
    }

    pub fn is_empty(&self) -> bool {
        self.chunks == 0
    }

    fn bit_position(index: usize) -> (usize, u8) {
        (index / 8, 1u8 << (index % 8))
    }

    /// Count verified chunks without constructing a list of missing indexes.
    /// Ignore unused bits in the final partially populated bitmap byte.
    pub fn completed_count(&self) -> usize {
        let full_bytes = self.chunks / 8;
        let count = popcount_full_bytes(&self.completed[..full_bytes]);
        let remainder = self.chunks % 8;
        if remainder == 0 {
            count
        } else {
            let mask = low_bit_mask(remainder);
            count + (self.completed[full_bytes] & mask).count_ones() as usize
        }
    }

    pub fn remaining_count(&self) -> usize {
        self.chunks - self.completed_count()
    }

    pub fn mark_complete(&mut self, index: usize) -> Result<(), &'static str> {
        if index >= self.chunks {
            return Err("chunk index out of bounds");
        }
        let (byte, mask) = Self::bit_position(index);
        self.completed[byte] |= mask;
        Ok(())
    }

    fn validate_index_batch(&self, indexes: &[usize]) -> Result<(), &'static str> {
        let mut previous = None;
        for &index in indexes {
            if index >= self.chunks || previous.is_some_and(|last| index <= last) {
                return Err("chunk batch indexes must be in bounds and strictly increasing");
            }
            previous = Some(index);
        }
        Ok(())
    }

    // Update a dense sorted batch once per bitmap byte, instead of reading
    // and writing the same byte for each of its eight completion flags.
    // Short sparse batches keep the direct branch-free single-bit path.
    fn apply_index_batch(&mut self, indexes: &[usize], complete: bool) {
        if indexes.len() < 32 {
            for &index in indexes {
                let (byte, bit) = Self::bit_position(index);
                if complete {
                    self.completed[byte] |= bit;
                } else {
                    self.completed[byte] &= !bit;
                }
            }
            return;
        }
        let mut current_byte = usize::MAX;
        let mut mask = 0u8;
        for &index in indexes {
            let (byte, bit) = Self::bit_position(index);
            if byte != current_byte {
                if current_byte != usize::MAX {
                    if complete {
                        self.completed[current_byte] |= mask;
                    } else {
                        self.completed[current_byte] &= !mask;
                    }
                }
                current_byte = byte;
                mask = 0;
            }
            mask |= bit;
        }
        if current_byte != usize::MAX {
            if complete {
                self.completed[current_byte] |= mask;
            } else {
                self.completed[current_byte] &= !mask;
            }
        }
    }

    /// Atomically record a sorted batch of verified, possibly nonadjacent
    /// chunks. No completion flags change if any index is invalid or repeated.
    pub fn mark_complete_indices(&mut self, indexes: &[usize]) -> Result<(), &'static str> {
        self.validate_index_batch(indexes)?;
        self.apply_index_batch(indexes, true);
        Ok(())
    }

    /// Requeue a sorted set of corrupt chunks while preserving all other
    /// verified progress. Validation runs before any checkpoint mutation.
    pub fn invalidate_indices(&mut self, indexes: &[usize]) -> Result<(), &'static str> {
        self.validate_index_batch(indexes)?;
        self.apply_index_batch(indexes, false);
        Ok(())
    }

    /// Apply an out-of-order batch only after validating every chunk index.
    /// Concurrent replicas can finish transfers in arbitrary order. A packed
    /// duplicate bitmap (at most 8 KiB) validates the entire batch before
    /// modifying any persistent completion bit.
    fn apply_unordered_indices(
        &mut self,
        indexes: &[usize],
        complete: bool,
    ) -> Result<(), &'static str> {
        // Small completion notifications are the hot path for asynchronous
        // replicas. Validate on the stack before changing any live bits.
        // Larger batches retain the linear packed-bitmap duplicate check.
        if indexes.len() <= 16 {
            let mut seen = [usize::MAX; 16];
            for (position, &index) in indexes.iter().enumerate() {
                if index >= self.chunks {
                    return Err("unordered chunk index out of bounds");
                }
                if seen[..position].contains(&index) {
                    return Err("duplicate chunk index in unordered batch");
                }
                seen[position] = index;
            }
            for &index in indexes {
                let (byte, mask) = Self::bit_position(index);
                if complete {
                    self.completed[byte] |= mask;
                } else {
                    self.completed[byte] &= !mask;
                }
            }
            return Ok(());
        }
        let mut changes = vec![0u8; self.completed.len()];
        for &index in indexes {
            if index >= self.chunks {
                return Err("unordered chunk index out of bounds");
            }
            let (byte, mask) = Self::bit_position(index);
            if changes[byte] & mask != 0 {
                return Err("duplicate chunk index in unordered batch");
            }
            changes[byte] |= mask;
        }
        if complete {
            for (current, mask) in self.completed.iter_mut().zip(changes) {
                *current |= mask;
            }
        } else {
            for (current, mask) in self.completed.iter_mut().zip(changes) {
                *current &= !mask;
            }
        }
        Ok(())
    }

    /// Atomically record verified chunk indices in arbitrary completion order.
    /// Duplicate and out-of-bounds indices reject the whole batch.
    pub fn mark_complete_unordered(&mut self, indexes: &[usize]) -> Result<(), &'static str> {
        self.apply_unordered_indices(indexes, true)
    }

    /// Atomically invalidate arbitrarily ordered chunk indices after a scrub.
    /// Other independently verified completion flags remain unchanged.
    pub fn invalidate_unordered(&mut self, indexes: &[usize]) -> Result<(), &'static str> {
        self.apply_unordered_indices(indexes, false)
    }

    /// Mark a half-open range of verified chunks complete in one operation.
    /// Fully covered bytes are updated in bulk instead of one bit at a time.
    pub fn mark_complete_range(&mut self, start: usize, end: usize) -> Result<(), &'static str> {
        if start > end || end > self.chunks {
            return Err("chunk range out of bounds");
        }
        let prefix_end = start.div_ceil(8).saturating_mul(8).min(end);
        for index in start..prefix_end {
            let (byte, mask) = Self::bit_position(index);
            self.completed[byte] |= mask;
        }
        let full_end = (end / 8) * 8;
        if prefix_end < full_end {
            self.completed[prefix_end / 8..full_end / 8].fill(u8::MAX);
        }
        for index in prefix_end.max(full_end)..end {
            let (byte, mask) = Self::bit_position(index);
            self.completed[byte] |= mask;
        }
        Ok(())
    }

    /// Requeue a half-open chunk range after failed checksum verification.
    /// Reset only the affected completion bits; unrelated verified ranges
    /// remain durable across checkpoint serialization and retry.
    pub fn invalidate_range(&mut self, start: usize, end: usize) -> Result<(), &'static str> {
        if start > end || end > self.chunks {
            return Err("chunk range out of bounds");
        }
        let prefix_end = start.div_ceil(8).saturating_mul(8).min(end);
        for index in start..prefix_end {
            let (byte, bit) = Self::bit_position(index);
            self.completed[byte] &= !bit;
        }
        let full_end = (end / 8) * 8;
        if prefix_end < full_end {
            self.completed[prefix_end / 8..full_end / 8].fill(0);
        }
        for index in prefix_end.max(full_end)..end {
            let (byte, bit) = Self::bit_position(index);
            self.completed[byte] &= !bit;
        }
        Ok(())
    }

    fn validate_runs(
        &self,
        runs: &[crate::distributed_chunk_plan::ChunkRun],
    ) -> Result<(), &'static str> {
        if runs.len() > 65_536 {
            return Err("too many transfer runs");
        }
        let mut previous_end = 0usize;
        for run in runs {
            let end = run
                .first_index
                .checked_add(run.chunk_count)
                .ok_or("transfer run index overflow")?;
            if run.chunk_count == 0 || run.first_index < previous_end || end > self.chunks {
                return Err("invalid or overlapping transfer run indexes");
            }
            previous_end = end;
        }
        Ok(())
    }

    /// Commit verified coalesced chunk runs without expanding individual
    /// bit indexes. Validate the entire batch before any checkpoint changes.
    /// Every constituent chunk must be independently verified by the caller.
    pub fn mark_complete_runs(
        &mut self,
        runs: &[crate::distributed_chunk_plan::ChunkRun],
    ) -> Result<(), &'static str> {
        self.validate_runs(runs)?;
        for run in runs {
            self.mark_complete_range(run.first_index, run.first_index + run.chunk_count)?;
        }
        Ok(())
    }

    /// Atomically requeue coalesced runs after failed transfer validation.
    /// Unrelated verified ranges remain complete.
    pub fn invalidate_runs(
        &mut self,
        runs: &[crate::distributed_chunk_plan::ChunkRun],
    ) -> Result<(), &'static str> {
        self.validate_runs(runs)?;
        for run in runs {
            self.invalidate_range(run.first_index, run.first_index + run.chunk_count)?;
        }
        Ok(())
    }

    /// Iterate missing chunk indexes in ascending order without materializing
    /// the entire transfer checkpoint. Only live bits in the last byte count.
    pub fn remaining_indices(&self) -> impl Iterator<Item = usize> + '_ {
        // Completed transfers have long stretches of full bytes. Check eight
        // bytes together and skip all 64 verified chunks in one iteration.
        // Use little-endian words to preserve chunk index order on all CPUs.
        // Count complete logical words, not allocated bytes: a final
        // 63-bit word contains one padding bit that must not be returned.
        let full_words = self.chunks / 64;
        let words = self.completed[..full_words * 8]
            .chunks_exact(8)
            .enumerate()
            .flat_map(|(word_index, bytes)| {
                let mut missing =
                    !u64::from_le_bytes(bytes.try_into().expect("exactly eight checkpoint bytes"));
                std::iter::from_fn(move || {
                    if missing == 0 {
                        return None;
                    }
                    let bit = missing.trailing_zeros() as usize;
                    missing &= missing - 1;
                    Some(word_index * 64 + bit)
                })
            });
        let tail_base = full_words * 64;
        let tail = self.completed[full_words * 8..]
            .iter()
            .enumerate()
            .flat_map(move |(byte_index, &completed)| {
                let first = tail_base + byte_index * 8;
                let mut missing = !completed;
                if first + 8 > self.chunks {
                    missing &= low_bit_mask(self.chunks - first);
                }
                std::iter::from_fn(move || {
                    if missing == 0 {
                        return None;
                    }
                    let bit = missing.trailing_zeros() as usize;
                    missing &= missing - 1;
                    Some(first + bit)
                })
            });
        words.chain(tail)
    }

    /// Iterate unfinished chunks at or beyond an inclusive cursor.
    ///
    /// Jump directly to the requested packed bitmap byte. This avoids
    /// rescanning a nearly completed transfer for each continuation page.
    pub fn remaining_indices_from(&self, start: usize) -> impl Iterator<Item = usize> + '_ {
        let start = start.min(self.chunks);
        // Skip directly to the cursor's 64-bit word, not just its byte.
        // Large mostly-complete checkpoints can then reject 64 verified
        // chunks with one word comparison on every continuation poll.
        let full_words = self.chunks / 64;
        let words = self.completed[..full_words * 8]
            .chunks_exact(8)
            .enumerate()
            .skip(start / 64)
            .flat_map(move |(word_index, bytes)| {
                let base = word_index * 64;
                let mut missing =
                    !u64::from_le_bytes(bytes.try_into().expect("full checkpoint word"));
                if base < start {
                    missing &= u64::MAX << (start - base);
                }
                std::iter::from_fn(move || {
                    if missing == 0 {
                        return None;
                    }
                    let bit = missing.trailing_zeros() as usize;
                    missing &= missing - 1;
                    Some(base + bit)
                })
            });
        // The partial tail may have padding bits. Mask those before
        // filtering the cursor so they never surface as missing chunks.
        let tail_base = full_words * 64;
        let skip_tail_bytes = start.saturating_sub(tail_base) / 8;
        let tail = self.completed[full_words * 8 + skip_tail_bytes..]
            .iter()
            .enumerate()
            .flat_map(move |(offset, &completed)| {
                let base = tail_base + (skip_tail_bytes + offset) * 8;
                let mut missing = !completed & low_bit_mask((self.chunks - base).min(8));
                if base < start {
                    missing &= u8::MAX << (start - base);
                }
                std::iter::from_fn(move || {
                    if missing == 0 {
                        return None;
                    }
                    let bit = missing.trailing_zeros() as usize;
                    missing &= missing - 1;
                    Some(base + bit)
                })
            });
        words.chain(tail)
    }

    pub fn remaining(&self) -> Vec<usize> {
        let mut missing = Vec::with_capacity(self.remaining_count());
        missing.extend(self.remaining_indices());
        missing
    }
}

impl ResumeMap {
    /// Keep only chunks independently verified by at least `required` checkpoints.
    ///
    /// Checkpoints must belong to the same immutable object and chunk layout.
    /// The caller is responsible for authenticating each map and object ID.
    /// A bit-sliced accumulator counts 8 independent chunks per byte and
    /// avoids allocating one counter for every possible chunk.
    pub fn quorum_verified(maps: &[&Self], required: usize) -> Result<Self, &'static str> {
        if maps.is_empty() || maps.len() > 64 || required == 0 || required > maps.len() {
            return Err("invalid checkpoint quorum");
        }
        let chunks = maps[0].chunks;
        if maps.iter().any(|map| map.chunks != chunks) {
            return Err("cannot quorum-check mismatched transfer layouts");
        }
        let mut result = Self::new(chunks)?;
        if required == 1 {
            for map in maps {
                for (out, &flags) in result.completed.iter_mut().zip(&map.completed) {
                    *out |= flags;
                }
            }
            return Ok(result);
        }
        if required == maps.len() {
            result.completed.fill(u8::MAX);
            for map in maps {
                for (out, &flags) in result.completed.iter_mut().zip(&map.completed) {
                    *out &= flags;
                }
            }
            if chunks % 8 != 0 {
                let last = result
                    .completed
                    .last_mut()
                    .expect("nonempty partial bitmap");
                *last &= low_bit_mask(chunks % 8);
            }
            return Ok(result);
        }

        // 64 votes require 7 planes. For each of the eight chunk positions
        // in one byte, ripple-carry addition counts votes in parallel.
        for (byte_index, out) in result.completed.iter_mut().enumerate() {
            let mut planes = [0u8; 7];
            for map in maps {
                let mut carry = map.completed[byte_index];
                for plane in &mut planes {
                    let next = *plane ^ carry;
                    carry &= *plane;
                    *plane = next;
                }
                debug_assert_eq!(carry, 0);
            }
            // Compare the seven packed counters against the scalar quorum
            // from most significant bit down, without per-chunk branches.
            let mut greater = 0u8;
            let mut equal = u8::MAX;
            for bit in (0..7).rev() {
                if required & (1 << bit) == 0 {
                    greater |= equal & planes[bit];
                    equal &= !planes[bit];
                } else {
                    equal &= planes[bit];
                }
            }
            *out = greater | equal;
        }
        if chunks % 8 != 0 {
            let last = result
                .completed
                .last_mut()
                .expect("nonempty partial bitmap");
            *last &= low_bit_mask(chunks % 8);
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "distributed_resume_map_tests/checkpoint_quorum_tests.rs"]
mod checkpoint_quorum_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/word_progress_tests.rs"]
mod word_progress_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/merge_checkpoint_tests.rs"]
mod merge_checkpoint_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/binary_checkpoint_tests.rs"]
mod binary_checkpoint_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/popcount_progress_tests.rs"]
mod popcount_progress_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/grouped_ascii_resume_decoder_tests.rs"]
mod grouped_ascii_resume_decoder_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/completion_tail_tests.rs"]
mod completion_tail_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/lazy_remaining_index_tests.rs"]
mod lazy_remaining_index_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/invalidation_checkpoint_tests.rs"]
mod invalidation_checkpoint_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/atomic_sparse_verified_batch_tests.rs"]
mod atomic_sparse_verified_batch_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/word_missing_chunk_iteration_tests.rs"]
mod word_missing_chunk_iteration_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/checked_checkpoint_tests.rs"]
mod checked_checkpoint_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/bitmap_byte_batch_regressions.rs"]
mod bitmap_byte_batch_regressions;

#[cfg(test)]
#[path = "distributed_resume_map_tests/dense_ascii_checkpoint_tests.rs"]
mod dense_ascii_checkpoint_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/authenticated_checkpoint_intersection_tests.rs"]
mod authenticated_checkpoint_intersection_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/checked_decode_allocation_regression_tests.rs"]
mod checked_decode_allocation_regression_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/checkpoint_cursor_tests.rs"]
mod checkpoint_cursor_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/word_cursor_checkpoint_tests.rs"]
mod word_cursor_checkpoint_tests;

impl ResumeMap {
    /// Number of unfinished chunks strictly before the given cursor.
    ///
    /// Counts packed words rather than generating missing index lists. The
    /// endpoint may equal len(), but must not exceed the checkpoint.
    pub fn missing_count_before(&self, end: usize) -> Result<usize, &'static str> {
        if end > self.chunks {
            return Err("resume cursor out of bounds");
        }
        let full_bytes = end / 8;
        let completed = popcount_full_bytes(&self.completed[..full_bytes])
            + if end % 8 == 0 {
                0
            } else {
                (self.completed[full_bytes] & low_bit_mask(end % 8)).count_ones() as usize
            };
        Ok(end - completed)
    }

    /// Return the index of the zero-based nth unfinished chunk.
    ///
    /// Full 64-bit words are skipped with one popcount. Only the chosen word
    /// walks individual set bits, so dense 65K-chunk checkpoints need not
    /// allocate or visit every completed chunk.
    pub fn nth_missing(&self, mut rank: usize) -> Option<usize> {
        let full_words = self.chunks / 64;
        for (word_index, bytes) in self.completed[..full_words * 8].chunks_exact(8).enumerate() {
            let mut missing =
                !u64::from_le_bytes(bytes.try_into().expect("eight checkpoint bytes"));
            let count = missing.count_ones() as usize;
            if rank >= count {
                rank -= count;
                continue;
            }
            for _ in 0..rank {
                missing &= missing - 1;
            }
            return Some(word_index * 64 + missing.trailing_zeros() as usize);
        }
        let tail_base = full_words * 64;
        for (byte_index, &completed) in self.completed[full_words * 8..].iter().enumerate() {
            let start = tail_base + byte_index * 8;
            let live = (self.chunks - start).min(8);
            let mut missing = !completed & low_bit_mask(live);
            let count = missing.count_ones() as usize;
            if rank >= count {
                rank -= count;
                continue;
            }
            for _ in 0..rank {
                missing &= missing - 1;
            }
            return Some(start + missing.trailing_zeros() as usize);
        }
        None
    }
}

impl ResumeMap {
    /// Select the nth unfinished transfer chunk at or after a continuation cursor.
    ///
    /// Skip entire completed words instead of scanning every individual chunk.
    /// The cursor is inclusive and the rank is zero-based within the suffix.
    pub fn nth_missing_from(
        &self,
        start: usize,
        mut rank: usize,
    ) -> Result<Option<usize>, &'static str> {
        if start > self.chunks {
            return Err("resume cursor out of bounds");
        }
        if start == self.chunks {
            return Ok(None);
        }
        let first_word = start / 64;
        let full_words = self.chunks / 64;
        for (index, bytes) in self.completed[first_word * 8..full_words * 8]
            .chunks_exact(8)
            .enumerate()
        {
            let word = first_word + index;
            let mut missing = !u64::from_le_bytes(bytes.try_into().expect("checkpoint word"));
            if index == 0 {
                missing &= u64::MAX << (start % 64);
            }
            let count = missing.count_ones() as usize;
            if rank >= count {
                rank -= count;
                continue;
            }
            for _ in 0..rank {
                missing &= missing - 1;
            }
            return Ok(Some(word * 64 + missing.trailing_zeros() as usize));
        }
        let tail_base = full_words * 64;
        for (byte_index, &completed) in self.completed[full_words * 8..].iter().enumerate() {
            let position = tail_base + byte_index * 8;
            let live = (self.chunks - position).min(8);
            let mut missing = !completed & low_bit_mask(live);
            if start > position {
                missing &= !low_bit_mask((start - position).min(8));
            }
            let count = missing.count_ones() as usize;
            if rank >= count {
                rank -= count;
                continue;
            }
            for _ in 0..rank {
                missing &= missing - 1;
            }
            return Ok(Some(position + missing.trailing_zeros() as usize));
        }
        Ok(None)
    }
}

#[cfg(test)]
#[path = "distributed_resume_map_tests/paged_resume_select_tests.rs"]
mod paged_resume_select_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/resume_rank_select_tests.rs"]
mod resume_rank_select_tests;

impl ResumeMap {
    /// Return a bounded page of chunk indexes on which two authenticated
    /// checkpoints disagree. This is a comparison, not permission to treat
    /// either checkpoint as verified for the other replica.
    ///
    /// Starting at a cursor operates on packed bytes and never materializes
    /// the complete set of changed chunks.
    pub fn differing_indices_from(
        &self,
        other: &Self,
        start: usize,
        max_results: usize,
    ) -> Result<Vec<usize>, &'static str> {
        if self.chunks != other.chunks || start > self.chunks {
            return Err("checkpoint comparison requires matching layouts and a valid cursor");
        }
        if max_results > 65_536 {
            return Err("checkpoint difference page exceeds limit");
        }
        if max_results == 0 || start == self.chunks {
            return Ok(Vec::new());
        }
        let first_byte = start / 8;
        let mut changes = Vec::with_capacity(max_results.min(self.chunks - start));
        for byte_index in first_byte..self.completed.len() {
            let mut changed = self.completed[byte_index] ^ other.completed[byte_index];
            if byte_index == first_byte {
                changed &= u8::MAX << (start % 8);
            }
            changed &= low_bit_mask((self.chunks - byte_index * 8).min(8));
            while changed != 0 {
                let bit = changed.trailing_zeros() as usize;
                changes.push(byte_index * 8 + bit);
                if changes.len() == max_results {
                    return Ok(changes);
                }
                changed &= changed - 1;
            }
        }
        Ok(changes)
    }
}

#[cfg(test)]
#[path = "distributed_resume_map_tests/checkpoint_difference_tests.rs"]
mod checkpoint_difference_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/atomic_chunk_run_progress_tests.rs"]
mod atomic_chunk_run_progress_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/unordered_atomic_batch_tests.rs"]
mod unordered_atomic_batch_tests;

#[cfg(test)]
#[path = "distributed_resume_map_tests/inline_unordered_batch_tests.rs"]
mod inline_unordered_batch_tests;
