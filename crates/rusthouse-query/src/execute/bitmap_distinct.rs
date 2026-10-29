//! Exact, bounded-memory DISTINCT for 32-bit dimensions.
//! Sparse prefixes store exact keys; dense prefixes use 8 KiB bitmaps.

use super::ExecutionError;
use std::collections::HashMap;

const WORDS_PER_BLOCK: usize = 1024;
const MAX_BLOCKS: usize = 32_768; // Limit occupied prefixes independently of representation.
const MAX_SPARSE_KEYS: usize = 131_072; // Bound HashSet memory across all prefixes.
const MAX_DENSE_BLOCKS: usize = 8_192; // 64 MiB of 8 KiB bitmaps.

#[derive(Clone)]
enum Block {
    Sparse(std::collections::HashSet<u16>),
    Dense(Box<[u64; WORDS_PER_BLOCK]>),
}

const DENSE_THRESHOLD: usize = 4096;

fn set_bitmap_bit_if_unset(bitmap: &mut [u64; WORDS_PER_BLOCK], suffix: u16) -> bool {
    let index = usize::from(suffix);
    let mask = 1u64 << (index % 64);
    let word = &mut bitmap[index / 64];
    let fresh = *word & mask == 0;
    *word |= mask;
    fresh
}

#[derive(Default, Clone)]
pub(super) struct BitmapDistinct32 {
    blocks: HashMap<u16, Block>,
    count: u64,
    sparse_keys: usize,
    dense_blocks: usize,
}

impl BitmapDistinct32 {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn insert(&mut self, key: u32) -> Result<(), ExecutionError> {
        let prefix = (key >> 16) as u16;
        let suffix = (key & 0xffff) as u16;
        if let Some(Block::Dense(bitmap)) = self.blocks.get_mut(&prefix) {
            if set_bitmap_bit_if_unset(bitmap.as_mut(), suffix) {
                self.count = self.count.checked_add(1).ok_or_else(|| {
                    ExecutionError::Invalid("distinct count overflow".into())
                })?;
            }
            return Ok(());
        }
        if self.blocks.len() >= MAX_BLOCKS && !self.blocks.contains_key(&prefix) {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) exceeded its 32768-prefix budget".into(),
            ));
        }
        if self.dense_blocks >= MAX_DENSE_BLOCKS
            && matches!(self.blocks.get(&prefix),
                Some(Block::Sparse(values))
                    if values.len() + 1 == DENSE_THRESHOLD && !values.contains(&suffix))
        {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) exceeded its dense-bitmap memory budget".into(),
            ));
        }
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
                    self.dense_blocks += 1;
                    let mut bitmap = Box::new([0u64; WORDS_PER_BLOCK]);
                    for &value in values.iter() {
                        set_bitmap_bit_if_unset(bitmap.as_mut(), value);
                    }
                    *block = Block::Dense(bitmap);
                }
                fresh
            }
            Block::Dense(bitmap) => {
                set_bitmap_bit_if_unset(bitmap.as_mut(), suffix)
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

    /// Merge exact partial DISTINCT states without enumerating dense 32-bit keys.
    /// Prefix-budget violations are rejected before changing the destination.
    pub(super) fn merge_from(&mut self, other: &Self) -> Result<(), ExecutionError> {
        let new_prefixes = other.blocks.keys()
            .filter(|prefix| !self.blocks.contains_key(prefix))
            .count();
        if self.blocks.len().checked_add(new_prefixes)
            .is_none_or(|n| n > MAX_BLOCKS)
        {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) exceeded its 32768-prefix budget".into(),
            ));
        }
        for (&prefix, incoming) in &other.blocks {
            let additions = if let Some(current) = self.blocks.get_mut(&prefix) {
                Self::merge_block(current, incoming)
            } else {
                let cardinality = Self::block_len(incoming);
                self.blocks.insert(prefix, incoming.clone());
                cardinality
            };
            self.count = self.count.checked_add(additions)
                .ok_or_else(|| ExecutionError::Invalid("distinct count overflow".into()))?;
        }
        Ok(())
    }

    fn block_len(block: &Block) -> u64 {
        match block {
            Block::Sparse(values) => values.len() as u64,
            Block::Dense(words) => words.iter().map(|word| u64::from(word.count_ones())).sum(),
        }
    }

    fn merge_block(current: &mut Block, incoming: &Block) -> u64 {
        // A dense source has at least 4096 keys. Converting a sparse
        // destination once allows 1024 word-wise ORs, not thousands of probes.
        if matches!(incoming, Block::Dense(_)) && matches!(current, Block::Sparse(_)) {
            let Block::Sparse(values) = current else { unreachable!() };
            let mut bitmap = Box::new([0u64; WORDS_PER_BLOCK]);
            for &suffix in values.iter() {
                set_bitmap_bit_if_unset(bitmap.as_mut(), suffix);
            }
            *current = Block::Dense(bitmap);
        }
        let additions = match (&mut *current, incoming) {
            (Block::Dense(left), Block::Dense(right)) => {
                let mut added = 0u64;
                for (a, b) in left.iter_mut().zip(right.iter()) {
                    added += u64::from((!(*a) & *b).count_ones());
                    *a |= *b;
                }
                added
            }
            (Block::Dense(left), Block::Sparse(right)) => {
                right.iter().filter(|&&suffix| {
                    set_bitmap_bit_if_unset(left.as_mut(), suffix)
                }).count() as u64
            }
            (Block::Sparse(left), Block::Sparse(right)) => {
                let old = left.len();
                left.extend(right.iter().copied());
                (left.len() - old) as u64
            }
            (Block::Sparse(_), Block::Dense(_)) => unreachable!("dense input converted above"),
        };
        if let Block::Sparse(values) = current {
            if values.len() >= DENSE_THRESHOLD {
                let mut bitmap = Box::new([0u64; WORDS_PER_BLOCK]);
                for &suffix in values.iter() {
                    set_bitmap_bit_if_unset(bitmap.as_mut(), suffix);
                }
                *current = Block::Dense(bitmap);
            }
        }
        additions
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
        for prefix in 0..40u32 {
            for suffix in 0..3200u32 {
                state.insert((prefix << 16) | suffix).unwrap();
            }
        }
        for suffix in 0..(super::MAX_SPARSE_KEYS - 40 * 3200) as u32 {
            state.insert((40 << 16) | suffix).unwrap();
        }
        assert_eq!(state.sparse_keys, super::MAX_SPARSE_KEYS);
        assert!(state.insert(0).is_ok());
        assert!(state.insert((42 << 16) | 7).is_err());
        assert_eq!(state.len(), super::MAX_SPARSE_KEYS as u64);
    }

    #[test]
    fn dense_bitmap_budget_rejects_new_allocations_without_mutation() {
        let mut state = BitmapDistinct32::new();
        for key in 0..4095u32 {
            state.insert(key).unwrap();
        }
        state.dense_blocks = super::MAX_DENSE_BLOCKS;
        assert!(state.insert(4095).is_err());
        assert_eq!(state.len(), 4095);
        assert!(state.insert(1).is_ok());
        assert_eq!(state.len(), 4095);
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
mod merge_tests {
    use super::*;

    #[test]
    fn sparse_and_dense_partial_aggregates_union_without_double_counting() {
        let mut left = BitmapDistinct32::new();
        let mut right = BitmapDistinct32::new();
        for id in 0..70_000u32 {
            left.insert(id).unwrap();
        }
        for id in 35_000..105_000u32 {
            right.insert(id).unwrap();
        }
        left.merge_from(&right).unwrap();
        assert_eq!(left.len(), 105_000);
        left.merge_from(&right).unwrap();
        assert_eq!(left.len(), 105_000);
        for id in [0, 35_000, 69_999, 70_000, 104_999] {
            left.insert(id).unwrap();
        }
        assert_eq!(left.len(), 105_000);
    }

    #[test]
    fn new_prefix_and_signed_key_masks_remain_exact() {
        let mut left = BitmapDistinct32::new();
        let mut right = BitmapDistinct32::new();
        for key in [i32::MIN, -1, 5] { left.insert(key as u32).unwrap(); }
        for key in [-1i32, 5, 6, i32::MAX] { right.insert(key as u32).unwrap(); }
        left.merge_from(&right).unwrap();
        assert_eq!(left.len(), 5);
    }

    #[test]
    fn prefix_budget_rejects_merge_without_partial_mutation() {
        let mut full = BitmapDistinct32::new();
        for prefix in 0..MAX_BLOCKS {
            full.insert((prefix as u32) << 16).unwrap();
        }
        let mut incoming = BitmapDistinct32::new();
        incoming.insert(0).unwrap();
        incoming.insert((MAX_BLOCKS as u32) << 16).unwrap();
        assert!(full.merge_from(&incoming).is_err());
        assert_eq!(full.len(), MAX_BLOCKS as u64);
        full.insert(0).unwrap();
        assert_eq!(full.len(), MAX_BLOCKS as u64);
    }
}
