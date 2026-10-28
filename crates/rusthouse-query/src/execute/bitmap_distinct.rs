//! Exact, bounded-memory DISTINCT for 32-bit dimensions.
//! Dense groups use one 8 KiB bitmap per observed high 16-bit prefix.

use super::ExecutionError;
use std::collections::HashMap;

const WORDS_PER_BLOCK: usize = 1024;
const MAX_BLOCKS: usize = 32_768; // 256 MiB for dense bitmap payloads.

#[derive(Default)]
pub(super) struct BitmapDistinct32 {
    blocks: HashMap<u16, Box<[u64; WORDS_PER_BLOCK]>>,
    count: u64,
}

impl BitmapDistinct32 {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn insert(&mut self, key: u32) -> Result<(), ExecutionError> {
        let prefix = (key >> 16) as u16;
        if self.blocks.len() >= MAX_BLOCKS && !self.blocks.contains_key(&prefix) {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) exceeded the 256 MiB bitmap budget".into(),
            ));
        }
        let block = self.blocks.entry(prefix)
            .or_insert_with(|| Box::new([0u64; WORDS_PER_BLOCK]));
        let low = (key & 0xffff) as usize;
        let bit = 1u64 << (low & 63);
        let word = &mut block[low >> 6];
        if *word & bit == 0 {
            *word |= bit;
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
