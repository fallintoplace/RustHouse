//! Exact, bounded-memory DISTINCT for 32-bit dimensions.
//! Sparse prefixes store exact keys; dense prefixes use 8 KiB bitmaps.

use super::ExecutionError;
use std::collections::HashMap;

const WORDS_PER_BLOCK: usize = 1024;
const MAX_BLOCKS: usize = 65_536; // Every UInt16 prefix can be represented exactly.
const MAX_PAYLOAD_BYTES: usize = 128 * 1024 * 1024; // Combined sparse and dense payload budget.

enum Block {
    Sparse(Vec<u16>),
    Dense(Box<[u64; WORDS_PER_BLOCK]>),
}

const DENSE_THRESHOLD: usize = 4096;

/// Approximate owned payload bytes. HashMap and Vec capacity add overhead,
/// so this is a conservative payload guard rather than an RSS guarantee.
fn within_payload_budget(dense_blocks: usize, sparse_entries: usize) -> bool {
    dense_blocks.checked_mul(WORDS_PER_BLOCK * std::mem::size_of::<u64>())
        .and_then(|used| sparse_entries.checked_mul(std::mem::size_of::<u16>())
            .and_then(|sparse| used.checked_add(sparse)))
        .is_some_and(|used| used <= MAX_PAYLOAD_BYTES)
}


fn insert_bitmap_bit(bitmap: &mut [u64; WORDS_PER_BLOCK], suffix: u16) -> bool {
    let index = usize::from(suffix);
    let mask = 1u64 << (index % 64);
    let word = &mut bitmap[index / 64];
    let fresh = *word & mask == 0;
    *word |= mask;
    fresh
}

#[derive(Default)]
pub(super) struct BitmapDistinct32 {
    blocks: HashMap<u16, Block>,
    count: u64,
    dense_blocks: usize,
    sparse_entries: usize,
}

impl BitmapDistinct32 {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn insert(&mut self, key: u32) -> Result<(), ExecutionError> {
        let prefix = (key >> 16) as u16;
        if self.blocks.len() >= MAX_BLOCKS && !self.blocks.contains_key(&prefix) {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) exceeded its UInt32 prefix budget".into(),
            ));
        }
        let suffix = (key & 0xffff) as u16;
        // Random high-cardinality keys spread across many sparse prefixes.
        // Sorted vectors are substantially smaller than per-prefix hash sets.
        let block = self.blocks.entry(prefix)
            .or_insert_with(|| Block::Sparse(Vec::new()));
        let inserted = match block {
            Block::Sparse(values) => match values.binary_search(&suffix) {
                Ok(_) => false,
                Err(position) => {
                    if values.len() + 1 == DENSE_THRESHOLD {
                        let new_sparse = self.sparse_entries - values.len();
                        if !within_payload_budget(self.dense_blocks + 1, new_sparse) {
                            return Err(ExecutionError::Invalid(
                                "COUNT(DISTINCT) exceeded its 128 MiB payload budget".into(),
                            ));
                        }
                        let mut bitmap = Box::new([0u64; WORDS_PER_BLOCK]);
                        for &value in values.iter() {
                            insert_bitmap_bit(bitmap.as_mut(), value);
                        }
                        insert_bitmap_bit(bitmap.as_mut(), suffix);
                        *block = Block::Dense(bitmap);
                        self.dense_blocks += 1;
                        self.sparse_entries = new_sparse;
                    } else {
                        if !within_payload_budget(self.dense_blocks, self.sparse_entries + 1) {
                            return Err(ExecutionError::Invalid(
                                "COUNT(DISTINCT) exceeded its 128 MiB payload budget".into(),
                            ));
                        }
                        values.insert(position, suffix);
                        self.sparse_entries += 1;
                    }
                    true
                }
            },
            Block::Dense(bitmap) => insert_bitmap_bit(bitmap.as_mut(), suffix),
        };
        if inserted {
            self.count = self.count.checked_add(1)
                .ok_or_else(|| ExecutionError::Invalid("distinct count overflow".into()))?;
        }
        Ok(())
    }

    pub(super) fn len(&self) -> u64 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_millions_of_rows_with_exact_32_bit_identity() {
        let mut bitmap = BitmapDistinct32::new();
        for value in 0..500_000u32 {
            bitmap.insert(value).unwrap();
            bitmap.insert(value).unwrap();
        }
        assert_eq!(bitmap.len(), 500_000);
        for value in [0, 65_535, 65_536, 499_999] {
            bitmap.insert(value).unwrap();
        }
        assert_eq!(bitmap.len(), 500_000);
    }

    #[test]
    fn preserves_signed_bit_patterns_and_cross_block_uniqueness() {
        let mut bitmap = BitmapDistinct32::new();
        for number in [i32::MIN, -1, 0, 1, i32::MAX] {
            bitmap.insert(number as u32).unwrap();
        }
        bitmap.insert((-1i32) as u32).unwrap();
        assert_eq!(bitmap.len(), 5);
    }
}

#[cfg(test)]
mod adaptive_large_keyspace_tests {
    use super::BitmapDistinct32;

    #[test]
    fn stores_sparse_32_bit_prefixes_without_dense_blocks() {
        let mut state = BitmapDistinct32::new();
        for prefix in 0..32_768u32 {
            state.insert(prefix << 16).unwrap();
        }
        assert_eq!(state.len(), 32_768);
        assert!(state.insert(0).is_ok());
        assert_eq!(state.len(), 32_768);
    }

    #[test]
    fn converts_dense_prefixes_without_losing_keys() {
        let mut state = BitmapDistinct32::new();
        for low in 0..5000u32 {
            state.insert((77 << 16) | low).unwrap();
        }
        for low in 0..5000u32 {
            state.insert((77 << 16) | low).unwrap();
        }
        assert_eq!(state.len(), 5000);
    }
}

#[cfg(test)]
mod full_domain_sparse_tests {
    use super::*;

    #[test]
    fn exact_random_like_uint32_keys_span_more_than_32768_prefixes() {
        let mut state = BitmapDistinct32::new();
        for prefix in 0..65_536u32 {
            state.insert((prefix << 16) | (prefix % 271)).unwrap();
        }
        assert_eq!(state.len(), 65_536);
        for prefix in 0..65_536u32 {
            state.insert((prefix << 16) | (prefix % 271)).unwrap();
        }
        assert_eq!(state.len(), 65_536);
    }

    #[test]
    fn ordered_sparse_insert_handles_duplicates_and_promotion() {
        let mut state = BitmapDistinct32::new();
        for suffix in (0..5000u32).rev() {
            state.insert((300 << 16) | suffix).unwrap();
        }
        assert_eq!(state.len(), 5000);
        for suffix in 0..5000u32 {
            state.insert((300 << 16) | suffix).unwrap();
        }
        assert_eq!(state.len(), 5000);
        assert_eq!(state.dense_blocks, 1);
    }
}

#[cfg(test)]
mod memory_budget_tests {
    use super::*;

    #[test]
    fn sparse_and_dense_payload_share_one_budget() {
        let dense_bytes = WORDS_PER_BLOCK * std::mem::size_of::<u64>();
        let max_dense = MAX_PAYLOAD_BYTES / dense_bytes;
        assert!(within_payload_budget(max_dense, 0));
        assert!(!within_payload_budget(max_dense, 1));
        assert!(within_payload_budget(0, MAX_PAYLOAD_BYTES / 2));
        assert!(!within_payload_budget(0, MAX_PAYLOAD_BYTES / 2 + 1));
    }

    #[test]
    fn promotion_accounts_for_previous_sparse_entries() {
        let mut state = BitmapDistinct32::new();
        for value in 0..DENSE_THRESHOLD as u32 {
            state.insert((19 << 16) | value).unwrap();
        }
        assert_eq!(state.dense_blocks, 1);
        assert_eq!(state.sparse_entries, 0);
        assert_eq!(state.len(), DENSE_THRESHOLD as u64);
    }
}
