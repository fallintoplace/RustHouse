//! Exact, bounded-memory DISTINCT for 32-bit dimensions.
//! Sparse prefixes store exact keys; dense prefixes use 8 KiB bitmaps.

use super::ExecutionError;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

const WORDS_PER_BLOCK: usize = 1024;
const MAX_BLOCKS: usize = 65_536; // All UInt16 prefixes, still bounded by sparse and dense memory caps.
const MAX_SPARSE_KEYS: usize = 131_072; // Bound HashSet memory across all prefixes.
const MAX_DENSE_BLOCKS: usize = 8_192; // 64 MiB of 8 KiB bitmaps.

#[derive(Clone)]
enum Block {
    // Most 32-bit dimensions occupy a prefix only once. Store the first
    // suffix inline instead of allocating a hash table per occupied prefix.
    Single(u16),
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
        let suffix = key as u16;
        let block_count = self.blocks.len();

        // A single prefix lookup serves both existing and new blocks. The
        // usual sparse insert needs no duplicate probe while budgets have room.
        let inserted = match self.blocks.entry(prefix) {
            Entry::Occupied(mut entry) => {
                let block = entry.get_mut();
                match block {
                    Block::Single(existing) => {
                        if *existing == suffix {
                            false
                        } else {
                            if self.sparse_keys >= MAX_SPARSE_KEYS {
                                return Err(ExecutionError::Invalid(
                                    "COUNT(DISTINCT) exceeded its sparse-key memory budget".into(),
                                ));
                            }
                            let mut values = std::collections::HashSet::with_capacity(2);
                            values.insert(*existing);
                            values.insert(suffix);
                            *block = Block::Sparse(values);
                            self.sparse_keys += 1;
                            true
                        }
                    }
                    Block::Dense(bitmap) => set_bitmap_bit_if_unset(bitmap.as_mut(), suffix),
                    Block::Sparse(values) => {
                        let becoming_dense = values.len() + 1 == DENSE_THRESHOLD;
                        // Only check for duplicates when a new key could exceed
                        // a budget. Existing keys must still be accepted.
                        if becoming_dense && self.dense_blocks >= MAX_DENSE_BLOCKS {
                            if values.contains(&suffix) {
                                return Ok(());
                            }
                            return Err(ExecutionError::Invalid(
                                "COUNT(DISTINCT) exceeded its dense-bitmap memory budget".into(),
                            ));
                        }
                        if self.sparse_keys >= MAX_SPARSE_KEYS && !becoming_dense {
                            if values.contains(&suffix) {
                                return Ok(());
                            }
                            return Err(ExecutionError::Invalid(
                                "COUNT(DISTINCT) exceeded its sparse-key memory budget".into(),
                            ));
                        }

                        let fresh = values.insert(suffix);
                        if fresh {
                            self.sparse_keys += 1;
                            if values.len() == DENSE_THRESHOLD {
                                self.sparse_keys -= DENSE_THRESHOLD;
                                self.dense_blocks += 1;
                                let mut bitmap = Box::new([0u64; WORDS_PER_BLOCK]);
                                for &value in values.iter() {
                                    set_bitmap_bit_if_unset(bitmap.as_mut(), value);
                                }
                                *block = Block::Dense(bitmap);
                            }
                        }
                        fresh
                    }
                }
            }
            Entry::Vacant(entry) => {
                if block_count >= MAX_BLOCKS {
                    return Err(ExecutionError::Invalid(
                        "COUNT(DISTINCT) exceeded its 65536-prefix budget".into(),
                    ));
                }
                if self.sparse_keys >= MAX_SPARSE_KEYS {
                    return Err(ExecutionError::Invalid(
                        "COUNT(DISTINCT) exceeded its sparse-key memory budget".into(),
                    ));
                }
                entry.insert(Block::Single(suffix));
                self.sparse_keys += 1;
                true
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
                "COUNT(DISTINCT) exceeded its 65536-prefix budget".into(),
            ));
        }
        // Plan memory before mutating any block. Partial states may each fit
        // their budgets while their union exceeds the global sparse/dense caps.
        let mut sparse_keys = self.sparse_keys;
        let mut dense_blocks = self.dense_blocks;
        for (&prefix, incoming) in &other.blocks {
            match (self.blocks.get(&prefix), incoming) {
                (None, Block::Single(_)) => sparse_keys += 1,
                (None, Block::Sparse(values)) => sparse_keys += values.len(),
                (None, Block::Dense(_)) => dense_blocks += 1,
                (Some(Block::Single(_)), Block::Dense(_)) => {
                    sparse_keys -= 1;
                    dense_blocks += 1;
                }
                (Some(Block::Sparse(current)), Block::Dense(_)) => {
                    sparse_keys -= current.len();
                    dense_blocks += 1;
                }
                (Some(Block::Single(current)), Block::Single(incoming)) => {
                    sparse_keys += usize::from(current != incoming);
                }
                (Some(Block::Single(current)), Block::Sparse(incoming)) => {
                    let added = incoming.len() - usize::from(incoming.contains(current));
                    if 1 + added >= DENSE_THRESHOLD {
                        sparse_keys -= 1;
                        dense_blocks += 1;
                    } else {
                        sparse_keys += added;
                    }
                }
                (Some(Block::Sparse(current)), Block::Single(incoming)) => {
                    let added = usize::from(!current.contains(incoming));
                    if current.len() + added >= DENSE_THRESHOLD {
                        sparse_keys -= current.len();
                        dense_blocks += 1;
                    } else {
                        sparse_keys += added;
                    }
                }
                (Some(Block::Sparse(current)), Block::Sparse(values)) => {
                    let added = values.iter().filter(|value| !current.contains(value)).count();
                    if current.len() + added >= DENSE_THRESHOLD {
                        sparse_keys -= current.len();
                        dense_blocks += 1;
                    } else {
                        sparse_keys += added;
                    }
                }
                (Some(Block::Dense(_)), _) => {}
            }
        }
        if sparse_keys > MAX_SPARSE_KEYS {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) exceeded its sparse-key memory budget".into(),
            ));
        }
        if dense_blocks > MAX_DENSE_BLOCKS {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) exceeded its dense-bitmap memory budget".into(),
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
        self.sparse_keys = sparse_keys;
        self.dense_blocks = dense_blocks;
        Ok(())
    }

    fn block_len(block: &Block) -> u64 {
        match block {
            Block::Single(_) => 1,
            Block::Sparse(values) => values.len() as u64,
            Block::Dense(words) => words.iter().map(|word| u64::from(word.count_ones())).sum(),
        }
    }

    fn merge_block(current: &mut Block, incoming: &Block) -> u64 {
        // A dense source has at least 4096 keys. Convert once and merge
        // entire bitmap words rather than probing individual source keys.
        if matches!(incoming, Block::Dense(_)) && !matches!(current, Block::Dense(_)) {
            let mut bitmap = Box::new([0u64; WORDS_PER_BLOCK]);
            match current {
                Block::Single(value) => {
                    set_bitmap_bit_if_unset(bitmap.as_mut(), *value);
                }
                Block::Sparse(values) => {
                    for &suffix in values.iter() {
                        set_bitmap_bit_if_unset(bitmap.as_mut(), suffix);
                    }
                }
                Block::Dense(_) => unreachable!(),
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
                right
                    .iter()
                    .filter(|&&suffix| set_bitmap_bit_if_unset(left.as_mut(), suffix))
                    .count() as u64
            }
            (Block::Dense(left), Block::Single(right)) => {
                u64::from(set_bitmap_bit_if_unset(left.as_mut(), *right))
            }
            (Block::Sparse(left), Block::Sparse(right)) => {
                let old = left.len();
                left.extend(right.iter().copied());
                (left.len() - old) as u64
            }
            (Block::Sparse(left), Block::Single(right)) => u64::from(left.insert(*right)),
            (Block::Single(left), Block::Single(right)) => {
                if left == right {
                    0
                } else {
                    let mut values = std::collections::HashSet::with_capacity(2);
                    values.insert(*left);
                    values.insert(*right);
                    *current = Block::Sparse(values);
                    1
                }
            }
            (Block::Single(left), Block::Sparse(right)) => {
                let additions = right.len() - usize::from(right.contains(left));
                if additions + 1 >= DENSE_THRESHOLD {
                    // A singleton plus this sparse state will be dense.
                    // Build its bitmap directly instead of cloning thousands
                    // of hash-set entries only to discard that set.
                    let mut bitmap = Box::new([0u64; WORDS_PER_BLOCK]);
                    for &suffix in right {
                        set_bitmap_bit_if_unset(bitmap.as_mut(), suffix);
                    }
                    set_bitmap_bit_if_unset(bitmap.as_mut(), *left);
                    *current = Block::Dense(bitmap);
                } else {
                    let mut values = right.clone();
                    values.insert(*left);
                    *current = Block::Sparse(values);
                }
                additions as u64
            }
            (Block::Single(_) | Block::Sparse(_), Block::Dense(_)) => {
                unreachable!("dense input converted above")
            }
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
mod sparse_insert_budget_tests {
    use super::*;

    #[test]
    fn sparse_to_dense_transition_is_allowed_at_the_sparse_budget() {
        let mut state = BitmapDistinct32::new();
        for prefix in 1..=31u32 {
            for suffix in 0..4000u32 {
                state.insert((prefix << 16) | suffix).unwrap();
            }
        }
        for suffix in 0..2977u32 {
            state.insert((32 << 16) | suffix).unwrap();
        }
        for suffix in 0..4095u32 {
            state.insert(suffix).unwrap();
        }
        assert_eq!(state.sparse_keys, MAX_SPARSE_KEYS);
        assert_eq!(state.dense_blocks, 0);

        state.insert(4095).unwrap();
        assert_eq!(state.len(), MAX_SPARSE_KEYS as u64 + 1);
        assert_eq!(state.sparse_keys, MAX_SPARSE_KEYS - DENSE_THRESHOLD + 1);
        assert_eq!(state.dense_blocks, 1);
        // The conversion frees sparse capacity for subsequent new prefixes.
        state.insert(33 << 16).unwrap();
        assert_eq!(state.len(), MAX_SPARSE_KEYS as u64 + 2);
    }

    #[test]
    fn dense_budget_rejection_leaves_near_threshold_block_unchanged() {
        let mut state = BitmapDistinct32::new();
        for suffix in 0..(DENSE_THRESHOLD - 1) as u32 {
            state.insert(suffix).unwrap();
        }
        state.dense_blocks = MAX_DENSE_BLOCKS;
        assert!(state.insert((DENSE_THRESHOLD - 1) as u32).is_err());
        assert_eq!(state.len(), (DENSE_THRESHOLD - 1) as u64);
        assert_eq!(state.sparse_keys, DENSE_THRESHOLD - 1);
        assert!(state.insert(10).is_ok());
        assert_eq!(state.len(), (DENSE_THRESHOLD - 1) as u64);
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
    fn full_uint32_prefix_space_is_exact_and_works_with_existing_merge() {
        let mut full = BitmapDistinct32::new();
        for prefix in 0..MAX_BLOCKS {
            full.insert((prefix as u32) << 16).unwrap();
        }
        for prefix in 0..MAX_BLOCKS {
            full.insert((prefix as u32) << 16).unwrap();
        }
        assert_eq!(full.len(), MAX_BLOCKS as u64);
        let mut incoming = BitmapDistinct32::new();
        incoming.insert(1).unwrap();
        incoming.insert(((MAX_BLOCKS - 1) as u32) << 16 | 1).unwrap();
        full.merge_from(&incoming).unwrap();
        assert_eq!(full.len(), MAX_BLOCKS as u64 + 2);
    }
}

#[cfg(test)]
mod partial_distinct_memory_budget_tests {
    use super::*;

    #[test]
    fn partial_merge_rejects_sparse_memory_overflow_without_mutation() {
        let mut left = BitmapDistinct32::new();
        let mut right = BitmapDistinct32::new();
        for prefix in 0..20_u32 {
            for suffix in 0..4000_u32 {
                left.insert(prefix << 16 | suffix).unwrap();
                right.insert((prefix + 20) << 16 | suffix).unwrap();
            }
        }
        assert!(left.merge_from(&right).is_err());
        assert_eq!(left.len(), 80_000);
        assert_eq!(left.sparse_keys, 80_000);
        assert_eq!(left.blocks.len(), 20);
    }

    #[test]
    fn merging_sparse_overlapping_prefixes_updates_dense_tracking() {
        let mut left = BitmapDistinct32::new();
        let mut right = BitmapDistinct32::new();
        for suffix in 0..3000_u32 {
            left.insert(suffix).unwrap();
        }
        for suffix in 2500..5000_u32 {
            right.insert(suffix).unwrap();
        }
        left.merge_from(&right).unwrap();
        assert_eq!(left.len(), 5000);
        assert_eq!(left.sparse_keys, 0);
        assert_eq!(left.dense_blocks, 1);
        left.insert(5000).unwrap();
        assert_eq!(left.len(), 5001);
    }
}

#[cfg(test)]
mod inline_singleton_distinct_tests {
    use super::*;

    #[test]
    fn wide_sparse_prefixes_do_not_allocate_one_hash_set_per_key() {
        let mut state = BitmapDistinct32::new();
        for prefix in 0..4096u32 {
            state.insert(prefix << 16).unwrap();
        }
        assert_eq!(state.len(), 4096);
        assert_eq!(state.sparse_keys, 4096);
        assert!(state.blocks.values().all(|block| matches!(block, Block::Single(_))));
        state.insert(0).unwrap();
        state.insert(1).unwrap();
        assert_eq!(state.len(), 4097);
        assert!(matches!(state.blocks.get(&0), Some(Block::Sparse(_))));
    }

    #[test]
    fn singleton_sparse_and_dense_merges_preserve_exact_counts() {
        let mut left = BitmapDistinct32::new();
        left.insert(0x0001_0001).unwrap();
        let mut right = BitmapDistinct32::new();
        right.insert(0x0001_0002).unwrap();
        right.insert(0x0001_0003).unwrap();
        left.merge_from(&right).unwrap();
        assert_eq!(left.len(), 3);
        left.merge_from(&right).unwrap();
        assert_eq!(left.len(), 3);

        let mut dense = BitmapDistinct32::new();
        for suffix in 0..4096u32 {
            dense.insert(0x0001_0000 | suffix).unwrap();
        }
        left.merge_from(&dense).unwrap();
        assert_eq!(left.len(), 4096);
        let mut other = BitmapDistinct32::new();
        other.insert(0x0001_1000).unwrap();
        other.merge_from(&left).unwrap();
        assert_eq!(other.len(), 4097);
        other.merge_from(&left).unwrap();
        assert_eq!(other.len(), 4097);
    }
}

#[cfg(test)]
mod singleton_dense_union_tests {
    use super::*;

    #[test]
    fn threshold_union_builds_the_exact_dense_bitmap() {
        let mut current = Block::Single(4_095);
        let incoming = Block::Sparse((0..4_095).collect());
        assert_eq!(BitmapDistinct32::merge_block(&mut current, &incoming), 4_095);
        let Block::Dense(bits) = current else { panic!("expected dense union"); };
        assert_eq!(bits.iter().map(|word| word.count_ones() as usize).sum::<usize>(), 4_096);
        assert_eq!(bits[63] & (1 << 63), 1 << 63);

        let mut duplicate = Block::Single(4_094);
        assert_eq!(BitmapDistinct32::merge_block(&mut duplicate, &incoming), 4_094);
        assert!(matches!(duplicate, Block::Sparse(_)));
    }
}
