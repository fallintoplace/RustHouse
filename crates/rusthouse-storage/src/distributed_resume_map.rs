//! Bounded completion map for retrying interrupted chunk transfers.

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

// Count completion flags a machine word at a time, rather than emitting
// one popcount for every byte in large resumable transfer checkpoints.
fn popcount_full_bytes(bytes: &[u8]) -> usize {
    let mut words = bytes.chunks_exact(8);
    let count = words.by_ref().map(|chunk| {
        u64::from_ne_bytes(chunk.try_into().expect("eight-byte chunk")).count_ones() as usize
    }).sum::<usize>();
    count + words.remainder().iter()
        .map(|byte| byte.count_ones() as usize)
        .sum::<usize>()
}

fn all_full_bytes_set(bytes: &[u8]) -> bool {
    let mut words = bytes.chunks_exact(8);
    words.by_ref().all(|chunk| {
        u64::from_ne_bytes(chunk.try_into().expect("eight-byte chunk")) == u64::MAX
    }) && words.remainder().iter().all(|&byte| byte == u8::MAX)
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

    /// Mark a half-open range of verified chunks complete in one operation.
    /// Fully covered bytes are updated in bulk instead of one bit at a time.
    pub fn mark_complete_range(
        &mut self,
        start: usize,
        end: usize,
    ) -> Result<(), &'static str> {
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
            .chunks_exact(8).enumerate().flat_map(|(word_index, bytes)| {
                let mut missing = !u64::from_le_bytes(
                    bytes.try_into().expect("exactly eight checkpoint bytes")
                );
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
            .iter().enumerate().flat_map(move |(byte_index, &completed)| {
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

    pub fn remaining(&self) -> Vec<usize> {
        let mut missing = Vec::with_capacity(self.remaining_count());
        missing.extend(self.remaining_indices());
        missing
    }

    pub fn complete(&self) -> bool {
        let full_bytes = self.chunks / 8;
        if !all_full_bytes_set(&self.completed[..full_bytes]) {
            return false;
        }
        let remainder = self.chunks % 8;
        if remainder == 0 {
            return true;
        }
        let mask = low_bit_mask(remainder);
        self.completed[full_bytes] & mask == mask
    }

    /// Stable ASCII encoding for persisting transfer progress.
    pub fn encode(&self) -> String {
        // Fill the output once. Only set completion positions, instead of
        // appending and masking each of the up to 65,536 input bits.
        let mut encoded = vec![b'0'; self.chunks];
        for (byte_index, &completed) in self.completed.iter().enumerate() {
            let mut bits = completed;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                let index = byte_index * 8 + bit;
                if index < self.chunks {
                    encoded[index] = b'1';
                }
                bits &= bits - 1;
            }
        }
        String::from_utf8(encoded).expect("binary map contains only ASCII digits")
    }

    pub fn decode(text: &str) -> Result<Self, &'static str> {
        let mut result = Self::new(text.len())?;
        for (byte_index, group) in text.as_bytes().chunks(8).enumerate() {
            let mut flags = 0u8;
            for (bit, &value) in group.iter().enumerate() {
                match value {
                    b'0' => {}
                    b'1' => flags |= 1u8 << bit,
                    _ => return Err("invalid resume map byte"),
                }
            }
            result.completed[byte_index] = flags;
        }
        Ok(result)
    }

    /// Merge verified progress for the same immutable object and chunk layout.
    /// The caller must authenticate that both maps refer to the same object.
    pub fn merge_verified(&mut self, other: &Self) -> Result<(), &'static str> {
        if self.chunks != other.chunks {
            return Err("cannot merge mismatched transfer layouts");
        }
        for (destination, &source) in self.completed.iter_mut().zip(&other.completed) {
            *destination |= source;
        }
        Ok(())
    }

    /// Write an authenticated-by-checksum checkpoint envelope.
    ///
    /// A flipped completion bit must never silently skip an unverified chunk.
    /// CRC32C detects accidental corruption, not malicious tampering. The
    /// original compact binary format remains available for compatibility.
    pub fn encode_binary_checked(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(12 + self.completed.len());
        encoded.extend_from_slice(b"RSM1");
        encoded.extend_from_slice(&(self.chunks as u32).to_le_bytes());
        encoded.extend_from_slice(&self.completed);
        let checksum = crate::distributed_crc32c::crc32c(&encoded);
        encoded.extend_from_slice(&checksum.to_le_bytes());
        encoded
    }

    /// Verify framing and checksum before trusting any completion flag.
    pub fn decode_binary_checked(encoded: &[u8]) -> Result<Self, &'static str> {
        if encoded.get(..4) != Some(b"RSM1".as_slice()) {
            return Err("invalid resume checkpoint magic");
        }
        let count = encoded.get(4..8).ok_or("truncated resume checkpoint")?;
        let chunks = u32::from_le_bytes(
            count.try_into().map_err(|_| "invalid resume checkpoint header")?,
        ) as usize;
        let map = Self::new(chunks)?;
        let body_end = 8 + map.completed.len();
        if encoded.len() != body_end + 4 {
            return Err("invalid resume checkpoint length");
        }
        let received = u32::from_le_bytes(
            encoded[body_end..body_end + 4].try_into()
                .map_err(|_| "invalid resume checkpoint checksum")?,
        );
        if crate::distributed_crc32c::crc32c(&encoded[..body_end]) != received {
            return Err("resume checkpoint checksum mismatch");
        }
        // Reuse the strict packed-bitmap decoder, including unused tail bits.
        Self::decode_binary(&encoded[4..body_end])
    }

    /// Serialize the exact chunk count and completion bitmap.
    /// Four little-endian count bytes are followed by the packed flags.
    pub fn encode_binary(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(4 + self.completed.len());
        encoded.extend_from_slice(&(self.chunks as u32).to_le_bytes());
        encoded.extend_from_slice(&self.completed);
        encoded
    }

    /// Reject truncated, oversized and noncanonical checkpoint bitmaps.
    pub fn decode_binary(bytes: &[u8]) -> Result<Self, &'static str> {
        let count = bytes.get(..4).ok_or("truncated resume checkpoint")?;
        let chunks = u32::from_le_bytes(
            count.try_into().map_err(|_| "invalid resume checkpoint header")?,
        ) as usize;
        let mut map = Self::new(chunks)?;
        if bytes.len() != 4 + map.completed.len() {
            return Err("invalid resume checkpoint length");
        }
        map.completed.copy_from_slice(&bytes[4..]);
        let tail = chunks % 8;
        if tail != 0 && map.completed.last().is_some_and(|flags| *flags >> tail != 0) {
            return Err("resume checkpoint has invalid tail bits");
        }
        Ok(map)
    }
}

#[cfg(test)]
mod word_progress_tests {
    use super::*;

    #[test]
    fn word_popcounts_and_full_checks_match_bitwise_reference() {
        for bits in [0, 1, 7, 8, 9, 63, 64, 65, 511, 512, 513, 65_535, 65_536] {
            let mut progress = ResumeMap::new(bits).unwrap();
            for index in 0..bits {
                if index % 7 != 2 && index % 11 != 3 {
                    progress.mark_complete(index).unwrap();
                }
            }
            let expected = (0..bits)
                .filter(|&index| index % 7 != 2 && index % 11 != 3)
                .count();
            assert_eq!(progress.completed_count(), expected, "bits={bits}");
            assert_eq!(progress.complete(), expected == bits, "bits={bits}");
            progress.mark_complete_range(0, bits).unwrap();
            assert_eq!(progress.completed_count(), bits);
            assert!(progress.complete());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_and_resumes_missing_chunks() {
        let mut map = ResumeMap::new(4).unwrap();
        map.mark_complete(0).unwrap();
        map.mark_complete(2).unwrap();
        let recovered = ResumeMap::decode(&map.encode()).unwrap();
        assert_eq!(recovered.remaining(), vec![1, 3]);
        assert!(!recovered.complete());
    }

    #[test]
    fn reports_chunk_progress_without_allocating_missing_indexes() {
        for chunks in [0usize, 1, 7, 8, 9, 17, 65_536] {
            let mut map = ResumeMap::new(chunks).unwrap();
            assert_eq!(map.completed_count(), 0);
            assert_eq!(map.remaining_count(), chunks);
            for index in (0..chunks).step_by(3) {
                map.mark_complete(index).unwrap();
                map.mark_complete(index).unwrap();
            }
            let complete = chunks.div_ceil(3);
            assert_eq!(map.completed_count(), complete);
            assert_eq!(map.remaining_count(), chunks - complete);
            assert_eq!(map.remaining().len(), map.remaining_count());
        }
    }

    #[test]
    fn range_completion_matches_individual_marks_at_byte_boundaries() {
        for len in [0usize, 1, 7, 8, 9, 16, 17, 31] {
            for start in 0..=len {
                for end in start..=len {
                    let mut bulk = ResumeMap::new(len).unwrap();
                    let mut separate = ResumeMap::new(len).unwrap();
                    bulk.mark_complete_range(start, end).unwrap();
                    for index in start..end {
                        separate.mark_complete(index).unwrap();
                    }
                    assert_eq!(bulk, separate, "{len} {start} {end}");
                }
            }
        }
        let mut map = ResumeMap::new(17).unwrap();
        assert!(map.mark_complete_range(1, 18).is_err());
        assert!(map.mark_complete_range(10, 9).is_err());
        assert_eq!(map.remaining(), (0..17).collect::<Vec<_>>());
        map.mark_complete_range(0, 17).unwrap();
        assert!(map.complete());
    }

    #[test]
    fn bitmap_roundtrips_across_byte_boundaries() {
        for chunks in [0usize, 1, 7, 8, 9, 15, 16, 17, 65_536] {
            let mut progress = ResumeMap::new(chunks).unwrap();
            assert_eq!(progress.len(), chunks);
            assert_eq!(progress.is_empty(), chunks == 0);
            for index in (0..chunks).step_by(3) {
                progress.mark_complete(index).unwrap();
            }
            let restored = ResumeMap::decode(&progress.encode()).unwrap();
            assert_eq!(restored, progress);
            assert_eq!(restored.remaining().len(), chunks - chunks.div_ceil(3));
            assert_eq!(restored.complete(), chunks == 0 || chunks == 1);
            for index in 0..chunks {
                progress.mark_complete(index).unwrap();
            }
            assert!(progress.complete());
            assert_eq!(progress.remaining(), Vec::<usize>::new());
        }
    }

    #[test]
    fn rejects_invalid_progress_and_bounds() {
        assert!(ResumeMap::new(65_537).is_err());
        assert!(ResumeMap::decode("10x01").is_err());
        assert!(ResumeMap::new(1).unwrap().mark_complete(2).is_err());
    }
    #[test]
    fn completion_checks_only_used_bits_in_partial_tail() {
        let mut progress = ResumeMap::new(9).unwrap();
        for index in 0..8 {
            progress.mark_complete(index).unwrap();
        }
        assert!(!progress.complete());
        progress.mark_complete(8).unwrap();
        assert!(progress.complete());
        assert!(ResumeMap::new(0).unwrap().complete());
    }

    #[test]
    fn remaining_ignores_unused_tail_bits() {
        for len in [1, 2, 7, 8, 9, 10, 15, 16, 17] {
            let mut progress = ResumeMap::new(len).unwrap();
            for index in (0..len).step_by(2) {
                progress.mark_complete(index).unwrap();
            }
            let expected = (0..len).filter(|index| index % 2 == 1).collect::<Vec<_>>();
            assert_eq!(progress.remaining(), expected);
        }
    }

    #[test]
    fn sparse_and_dense_ascii_maps_match_the_bitmap_reference() {
        for count in [0, 1, 7, 8, 9, 513, 65_536] {
            for step in [1, 2, 7, 31] {
                let mut map = ResumeMap::new(count).unwrap();
                for index in (0..count).step_by(step) {
                    map.mark_complete(index).unwrap();
                }
                let expected = (0..count).map(|index| {
                    let (byte, mask) = ResumeMap::bit_position(index);
                    if map.completed[byte] & mask == 0 { '0' } else { '1' }
                }).collect::<String>();
                assert_eq!(map.encode(), expected);
            }
        }
    }

    #[test]
    fn encoder_keeps_partial_byte_flags_in_index_order() {
        let mut map = ResumeMap::new(11).unwrap();
        for index in [0, 2, 7, 8, 10] {
            map.mark_complete(index).unwrap();
        }
        assert_eq!(map.encode(), "10100001101");
    }

    #[test]
    fn decoding_large_bitmaps_keeps_exact_bit_positions() {
        let input = "10100111".repeat(8192);
        let decoded = ResumeMap::decode(&input).unwrap();
        assert_eq!(decoded.len(), 65_536);
        assert_eq!(decoded.encode(), input);
        assert_eq!(decoded.remaining().len(), 3 * 8192);
    }

    #[test]
    fn partial_bitmap_reserves_exact_remaining_slots() {
        let mut progress = ResumeMap::new(65_536).unwrap();
        for index in (0..65_536).step_by(2) {
            progress.mark_complete(index).unwrap();
        }
        let remaining = progress.remaining();
        assert_eq!(remaining.len(), 32_768);
        assert_eq!(remaining.capacity(), remaining.len());
        assert_eq!(remaining[0], 1);
        assert_eq!(*remaining.last().unwrap(), 65_535);
    }
}

#[cfg(test)]
mod merge_checkpoint_tests {
    use super::*;

    #[test]
    fn combines_worker_progress_without_losing_bit_positions() {
        let mut first = ResumeMap::new(10).unwrap();
        first.mark_complete(0).unwrap();
        first.mark_complete(8).unwrap();
        let mut second = ResumeMap::new(10).unwrap();
        second.mark_complete(1).unwrap();
        second.mark_complete(9).unwrap();
        first.merge_verified(&second).unwrap();
        assert_eq!(first.remaining(), vec![2, 3, 4, 5, 6, 7]);
        assert_eq!(ResumeMap::decode(&first.encode()).unwrap(), first);
        first.merge_verified(&second).unwrap();
        assert_eq!(first.remaining().len(), 6);
    }

    #[test]
    fn incompatible_maps_leave_progress_unchanged() {
        let mut first = ResumeMap::new(8).unwrap();
        first.mark_complete(7).unwrap();
        let original = first.clone();
        assert!(first.merge_verified(&ResumeMap::new(9).unwrap()).is_err());
        assert_eq!(first, original);
        assert!(ResumeMap::new(0).unwrap().merge_verified(&ResumeMap::new(0).unwrap()).is_ok());
    }
}

#[cfg(test)]
mod binary_checkpoint_tests {
    use super::*;

    #[test]
    fn compact_checkpoints_round_trip_at_boundary_sizes() {
        for count in [0, 1, 7, 8, 9, 16, 65_536] {
            let mut map = ResumeMap::new(count).unwrap();
            for index in (0..count).step_by(3) {
                map.mark_complete(index).unwrap();
            }
            let encoded = map.encode_binary();
            assert_eq!(encoded.len(), 4 + count.div_ceil(8));
            assert_eq!(ResumeMap::decode_binary(&encoded).unwrap(), map);
        }
    }

    #[test]
    fn refuses_incorrect_lengths_counts_and_unused_tail_bits() {
        assert!(ResumeMap::decode_binary(&[1, 0, 0]).is_err());
        assert!(ResumeMap::decode_binary(&[1, 0, 0, 0]).is_err());
        assert!(ResumeMap::decode_binary(&[1, 0, 0, 0, 0x80]).is_err());
        assert!(ResumeMap::decode_binary(&[1, 0, 0, 0, 1, 0]).is_err());
        assert!(ResumeMap::decode_binary(&[1, 0, 1, 0]).is_err());
    }
}

#[cfg(test)]
mod popcount_progress_tests {
    use super::*;

    #[test]
    fn packed_popcounts_match_each_bit_across_partial_tail_bytes() {
        for chunks in [0, 1, 2, 7, 8, 9, 15, 16, 17, 63, 64, 65, 65_536] {
            let mut progress = ResumeMap::new(chunks).unwrap();
            for index in 0..chunks {
                if index % 3 != 1 { progress.mark_complete(index).unwrap(); }
            }
            let expected = (0..chunks).filter(|&index| {
                let (byte, mask) = ResumeMap::bit_position(index);
                progress.completed[byte] & mask != 0
            }).count();
            assert_eq!(progress.completed_count(), expected);
        }
    }
}

#[cfg(test)]
mod grouped_ascii_resume_decoder_tests {
    use super::*;

    #[test]
    fn decodes_bit_patterns_in_eight_character_groups() {
        for length in [0, 1, 7, 8, 9, 15, 16, 17, 511, 512, 513, 65_536] {
            let encoded = (0..length).map(|index| if index % 5 < 2 { '1' } else { '0' }).collect::<String>();
            let map = ResumeMap::decode(&encoded).unwrap();
            assert_eq!(map.encode(), encoded);
            assert_eq!(map.completed_count(), (0..length).filter(|i| i % 5 < 2).count());
        }
        for encoded in ["10x", "00000000x", "11111111\u{00a0}"] {
            assert!(ResumeMap::decode(encoded).is_err());
        }
    }
}

#[cfg(test)]
mod completion_tail_tests {
    use super::*;

    #[test]
    fn completion_handles_all_partial_bitmap_widths() {
        for length in 1..8 {
            let mut map = ResumeMap::new(length).unwrap();
            assert!(!map.complete());
            map.mark_complete_range(0, length - 1).unwrap();
            assert!(!map.complete());
            map.mark_complete(length - 1).unwrap();
            assert!(map.complete());
        }
    }
}

#[cfg(test)]
mod lazy_remaining_index_tests {
    use super::*;

    #[test]
    fn iterator_matches_materialized_progress_for_partial_and_full_bytes() {
        for chunks in [0, 1, 7, 8, 9, 63, 64, 65, 513, 65_536] {
            let mut map = ResumeMap::new(chunks).unwrap();
            assert_eq!(map.remaining_indices().collect::<Vec<_>>(), (0..chunks).collect::<Vec<_>>());
            for i in (0..chunks).step_by(3) {
                map.mark_complete(i).unwrap();
            }
            let remaining = map.remaining_indices().collect::<Vec<_>>();
            assert_eq!(remaining, map.remaining());
            assert_eq!(remaining.len(), map.remaining_count());
            map.mark_complete_range(0, chunks).unwrap();
            assert_eq!(map.remaining_indices().count(), 0);
        }
    }
}

#[cfg(test)]
mod invalidation_checkpoint_tests {
    use super::*;

    #[test]
    fn failed_chunk_range_is_requeued_without_losing_verified_neighbors() {
        let mut map = ResumeMap::new(17).unwrap();
        map.mark_complete_range(0, 17).unwrap();
        assert!(map.complete());
        map.invalidate_range(5, 13).unwrap();
        assert_eq!(map.remaining(), (5..13).collect::<Vec<_>>());
        assert_eq!(map.completed_count(), 9);
        let restored = ResumeMap::decode_binary(&map.encode_binary()).unwrap();
        assert_eq!(restored, map);
        let mut restored = restored;
        restored.mark_complete_range(5, 13).unwrap();
        assert!(restored.complete());
    }

    #[test]
    fn invalidation_is_exact_across_byte_boundaries() {
        for len in [0usize, 1, 7, 8, 9, 16, 17, 31, 63, 64, 65, 1024, 65_536] {
            let all = (0..len).filter(|index| index % 3 != 2).collect::<Vec<_>>();
            for &(start, end) in &[(0, 0), (0, len), (len / 3, len / 2), (len, len)] {
                let mut progress = ResumeMap::new(len).unwrap();
                for &index in &all { progress.mark_complete(index).unwrap(); }
                progress.invalidate_range(start, end).unwrap();
                let expected = (0..len)
                    .filter(|&index| index % 3 == 2 || (start..end).contains(&index))
                    .collect::<Vec<_>>();
                assert_eq!(progress.remaining(), expected, "{len} {start} {end}");
            }
        }
    }

    #[test]
    fn out_of_bounds_invalidation_never_changes_checkpoint() {
        let mut map = ResumeMap::new(9).unwrap();
        map.mark_complete_range(0, 9).unwrap();
        let original = map.encode_binary();
        assert!(map.invalidate_range(10, 10).is_err());
        assert!(map.invalidate_range(7, 6).is_err());
        assert!(map.invalidate_range(0, 10).is_err());
        assert_eq!(map.encode_binary(), original);
    }
}

#[cfg(test)]
mod word_missing_chunk_iteration_tests {
    use super::*;

    #[test]
    fn sparse_holes_cross_word_and_partial_byte_boundaries() {
        for len in [0usize, 1, 7, 8, 9, 63, 64, 65, 71, 72, 73, 511, 512, 513, 65_535, 65_536] {
            let mut map = ResumeMap::new(len).unwrap();
            map.mark_complete_range(0, len).unwrap();
            let holes = [0usize, 7, 8, 31, 32, 63, 64, 65, 71, 72, 127, 128, 511, 512, 65_534, 65_535];
            let expected = holes.iter().copied().filter(|&index| index < len).collect::<Vec<_>>();
            for &index in &expected {
                map.invalidate_range(index, index + 1).unwrap();
            }
            assert_eq!(map.remaining_indices().collect::<Vec<_>>(), expected, "len={len}");
            assert_eq!(map.remaining_count(), expected.len());
            assert_eq!(map.remaining(), expected);
            assert_eq!(map.remaining_indices().next(), expected.first().copied());
            assert_eq!(
                ResumeMap::decode_binary(&map.encode_binary()).unwrap().remaining(),
                expected,
            );
        }
    }

    #[test]
    fn partial_map_iterators_match_scalar_reference_for_dense_checkpoints() {
        for len in [63usize, 64, 65, 511, 512, 513, 8_191, 8_192, 65_536] {
            let mut map = ResumeMap::new(len).unwrap();
            for index in 0..len {
                if index % 7 != 0 && index % 101 != 0 {
                    map.mark_complete(index).unwrap();
                }
            }
            let expected = (0..len)
                .filter(|&index| index % 7 == 0 || index % 101 == 0)
                .collect::<Vec<_>>();
            assert_eq!(map.remaining_indices().collect::<Vec<_>>(), expected);
        }
    }
}

#[cfg(test)]
mod checked_checkpoint_tests {
    use super::*;

    #[test]
    fn versioned_checkpoints_round_trip_at_every_bitmap_boundary() {
        for chunks in [0, 1, 7, 8, 9, 63, 64, 65, 511, 512, 513, 65_535, 65_536] {
            let mut map = ResumeMap::new(chunks).unwrap();
            for index in (0..chunks).step_by(3) {
                map.mark_complete(index).unwrap();
            }
            let encoded = map.encode_binary_checked();
            assert_eq!(&encoded[..4], b"RSM1");
            assert_eq!(encoded.len(), 12 + chunks.div_ceil(8));
            assert_eq!(ResumeMap::decode_binary_checked(&encoded).unwrap(), map);
            assert_eq!(ResumeMap::decode_binary(&map.encode_binary()).unwrap(), map);
        }
    }

    #[test]
    fn every_single_bit_corruption_rejects_before_resuming_work() {
        for chunks in [1, 8, 9, 1024] {
            let mut map = ResumeMap::new(chunks).unwrap();
            for index in (0..chunks).step_by(2) {
                map.mark_complete(index).unwrap();
            }
            let encoded = map.encode_binary_checked();
            for position in 0..encoded.len() {
                for bit in 0..8 {
                    let mut corrupted = encoded.clone();
                    corrupted[position] ^= 1 << bit;
                    assert!(ResumeMap::decode_binary_checked(&corrupted).is_err(),
                        "chunks={chunks}, position={position}, bit={bit}");
                }
            }
        }
    }

    #[test]
    fn length_magic_and_unused_tail_bits_are_strictly_validated() {
        let map = ResumeMap::new(9).unwrap();
        let encoded = map.encode_binary_checked();
        assert!(ResumeMap::decode_binary_checked(&encoded[..encoded.len() - 1]).is_err());
        let mut extra = encoded.clone();
        extra.push(0);
        assert!(ResumeMap::decode_binary_checked(&extra).is_err());
        let mut wrong = encoded.clone();
        wrong[..4].copy_from_slice(b"RSM2");
        assert!(ResumeMap::decode_binary_checked(&wrong).is_err());
        let mut bad_tail = encoded.clone();
        bad_tail[9] = 0x80;
        let crc = crate::distributed_crc32c::crc32c(&bad_tail[..bad_tail.len() - 4]);
        let length = bad_tail.len();
        bad_tail[length - 4..].copy_from_slice(&crc.to_le_bytes());
        assert!(ResumeMap::decode_binary_checked(&bad_tail).is_err());
        assert!(ResumeMap::decode_binary_checked(b"").is_err());
    }
}
