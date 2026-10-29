//! Exact, bounded-memory DISTINCT for 32-bit dimensions.
//! Sparse prefixes store exact keys; dense prefixes use 8 KiB bitmaps.

use super::ExecutionError;
use std::collections::HashMap;

const WORDS_PER_BLOCK: usize = 1024;
const MAX_BLOCKS: usize = 32_768; // Limit occupied prefixes independently of representation.
const MAX_SPARSE_KEYS: usize = 131_072; // Bound HashSet memory across all prefixes.

enum Block {
    Sparse(std::collections::HashSet<u16>),
    Dense(Box<[u64; WORDS_PER_BLOCK]>),
}

const DENSE_THRESHOLD: usize = 4096;

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
    sparse_keys: usize,
}

impl BitmapDistinct32 {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn insert(&mut self, key: u32) -> Result<(), ExecutionError> {
        let prefix = (key >> 16) as u16;
        if self.blocks.len() >= MAX_BLOCKS && !self.blocks.contains_key(&prefix) {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) exceeded its 32768-prefix budget".into(),
            ));
        }
        let suffix = (key & 0xffff) as u16;
        // Do not admit new sparse entries once the global memory budget is
        // exhausted. Dense bitmap inserts and duplicate keys remain valid.
        if self.sparse_keys >= MAX_SPARSE_KEYS {
            match self.blocks.get(&prefix) {
                Some(Block::Dense(_)) => {}
                Some(Block::Sparse(values))
                    if values.contains(&suffix) || values.len() + 1 == DENSE_THRESHOLD => {}
                _ => return Err(ExecutionError::Invalid(
                    "COUNT(DISTINCT) exceeded its sparse-key memory budget".into(),
                )),
            }
        }
        let block = self.blocks.entry(prefix)
            .or_insert_with(|| Block::Sparse(std::collections::HashSet::new()));
        let inserted = match block {
            Block::Sparse(values) => {
                let fresh = values.insert(suffix);
                if fresh {
                    self.sparse_keys += 1;
                }
                if fresh && values.len() == DENSE_THRESHOLD {
                    self.sparse_keys -= DENSE_THRESHOLD;
                    let mut bitmap = Box::new([0u64; WORDS_PER_BLOCK]);
                    for &value in values.iter() {
                        insert_bitmap_bit(bitmap.as_mut(), value);
                    }
                    *block = Block::Dense(bitmap);
                }
                fresh
            }
            Block::Dense(bitmap) => {
                insert_bitmap_bit(bitmap.as_mut(), suffix)
            }
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
    fn bounds_sparse_distinct_memory_and_keeps_duplicates_valid() {
        let mut state = BitmapDistinct32::new();
        // Use 3200 entries per block to remain below the dense threshold.
        for prefix in 0..41u32 {
            for suffix in 0..3200u32 {
                state.insert((prefix << 16) | suffix).unwrap();
            }
        }
        for suffix in 0..(MAX_SPARSE_KEYS - 41 * 3200) as u32 {
            state.insert((41 << 16) | suffix).unwrap();
        }
        assert_eq!(state.sparse_keys, MAX_SPARSE_KEYS);
        assert!(state.insert(0).is_ok());
        assert!(state.insert((42 << 16) | 7).is_err());
        assert_eq!(state.len(), MAX_SPARSE_KEYS as u64);
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
