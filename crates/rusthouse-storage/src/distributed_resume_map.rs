//! Bounded completion map for retrying interrupted chunk transfers.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeMap {
    // Bit-packed completion flags: 8 times smaller than Vec<bool> when every
    // Rust bool occupies a byte, with the original ASCII encoding retained.
    chunks: usize,
    completed: Vec<u8>,
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

    pub fn mark_complete(&mut self, index: usize) -> Result<(), &'static str> {
        if index >= self.chunks {
            return Err("chunk index out of bounds");
        }
        let (byte, mask) = Self::bit_position(index);
        self.completed[byte] |= mask;
        Ok(())
    }

    pub fn remaining(&self) -> Vec<usize> {
        // Fresh transfers have no completed bits: construct the complete
        // index range with its final allocation size in one pass.
        if self.completed.iter().all(|&byte| byte == 0) {
            return (0..self.chunks).collect();
        }
        let mut missing = Vec::new();
        for (byte_index, &completed) in self.completed.iter().enumerate() {
            let mut bits = !completed;
            if byte_index == self.completed.len() - 1 && self.chunks % 8 != 0 {
                bits &= ((1u16 << (self.chunks % 8)) - 1) as u8;
            }
            while bits != 0 {
                missing.push(byte_index * 8 + bits.trailing_zeros() as usize);
                bits &= bits - 1;
            }
        }
        missing
    }

    pub fn complete(&self) -> bool {
        let full_bytes = self.chunks / 8;
        if !self.completed[..full_bytes]
            .iter()
            .all(|&byte| byte == u8::MAX)
        {
            return false;
        }
        let remainder = self.chunks % 8;
        remainder == 0
            || self.completed[full_bytes] & ((1u8 << remainder) - 1) == (1u8 << remainder) - 1
    }

    /// Stable ASCII encoding for persisting transfer progress.
    pub fn encode(&self) -> String {
        let mut encoded = String::with_capacity(self.chunks);
        for (byte_index, &bits) in self.completed.iter().enumerate() {
            let count = (self.chunks - byte_index * 8).min(8);
            for index in 0..count {
                encoded.push(if bits & (1 << index) != 0 { '1' } else { '0' });
            }
        }
        encoded
    }

    pub fn decode(text: &str) -> Result<Self, &'static str> {
        let mut result = Self::new(text.len())?;
        for (index, byte) in text.bytes().enumerate() {
            match byte {
                b'0' => {}
                b'1' => result.completed[index / 8] |= 1u8 << (index % 8),
                _ => return Err("invalid resume map byte"),
            }
        }
        Ok(result)
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
}

impl ResumeMap {
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
