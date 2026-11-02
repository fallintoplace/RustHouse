//! Sliding receive window for replica-transfer chunk acknowledgements.
//! Only externally verified checksums may be recorded as receipts.
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkReceipts {
    total: u64,
    next: u64,
    window: u64,
    buffered: BTreeMap<u64, u32>,
}

impl ChunkReceipts {
    pub fn new(total: u64, window: u64) -> Result<Self, &'static str> {
        if total == 0 || window == 0 || window > 4096 {
            return Err("invalid replica receive window");
        }
        Ok(Self { total, next: 0, window, buffered: BTreeMap::new() })
    }

    pub fn contiguous_received(&self) -> u64 { self.next }
    pub fn complete(&self) -> bool { self.next == self.total }
    pub fn buffered_receipts(&self) -> usize { self.buffered.len() }

    /// A matching duplicate inside the active window is idempotent.
    /// An old, already-finalized index cannot be revalidated from the compact
    /// window alone and must be rejected instead of silently accepted.
    pub fn acknowledge(&mut self, index: u64, crc32c: u32) -> Result<u64, &'static str> {
        let end = self.next.saturating_add(self.window).min(self.total);
        if index < self.next {
            return Err("receipt predates finalized transfer cursor");
        }
        if index >= end {
            return Err("receipt is outside the active transfer window");
        }
        if let Some(&known) = self.buffered.get(&index) {
            if known != crc32c {
                return Err("conflicting transfer chunk checksum");
            }
            return Ok(self.next);
        }
        self.buffered.insert(index, crc32c);
        while self.buffered.remove(&self.next).is_some() {
            self.next += 1;
        }
        Ok(self.next)
    }

    /// Missing half-open chunk intervals within the current receive window.
    pub fn missing_ranges(&self) -> Vec<Range<u64>> {
        let end = self.next.saturating_add(self.window).min(self.total);
        let mut start = None;
        let mut ranges = Vec::new();
        for index in self.next..end {
            if !self.buffered.contains_key(&index) {
                start.get_or_insert(index);
            } else if let Some(begin) = start.take() {
                ranges.push(begin..index);
            }
        }
        if let Some(begin) = start { ranges.push(begin..end); }
        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_receipts_do_not_skip_holes() {
        let mut window = ChunkReceipts::new(5, 3).unwrap();
        assert_eq!(window.acknowledge(2, 99).unwrap(), 0);
        assert_eq!(window.missing_ranges(), vec![0..2]);
        assert_eq!(window.acknowledge(0, 10).unwrap(), 1);
        assert_eq!(window.acknowledge(1, 11).unwrap(), 3);
        assert_eq!(window.acknowledge(4, 40).unwrap(), 3);
        assert_eq!(window.acknowledge(3, 30).unwrap(), 5);
        assert!(window.complete());
    }

    #[test]
    fn duplicate_mismatch_is_rejected_without_mutation() {
        let mut window = ChunkReceipts::new(100, 4).unwrap();
        window.acknowledge(1, 123).unwrap();
        assert_eq!(window.acknowledge(1, 123).unwrap(), 0);
        let original = window.clone();
        assert!(window.acknowledge(1, 456).is_err());
        assert!(window.acknowledge(4, 4).is_err());
        assert_eq!(window, original);
    }

    #[test]
    fn finalized_chunks_and_invalid_windows_are_rejected() {
        assert!(ChunkReceipts::new(0, 1).is_err());
        assert!(ChunkReceipts::new(1, 0).is_err());
        let mut window = ChunkReceipts::new(1, 1).unwrap();
        window.acknowledge(0, 0).unwrap();
        assert!(window.acknowledge(0, 0).is_err());
        assert_eq!(window.missing_ranges(), vec![]);
    }
}
