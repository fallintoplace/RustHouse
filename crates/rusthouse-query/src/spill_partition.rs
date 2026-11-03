//! Stable key partitioning for a future disk-backed hash join.
//! This module does not yet change the query executor's JOIN behavior.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Pick a partition without depending on randomized HashMap seeds.
/// Callers must encode typed keys unambiguously before hashing.
pub fn spill_partition_for_key(key: &[u8], partitions: usize) -> Result<usize, &'static str> {
    if partitions == 0 || partitions > 1024 || !partitions.is_power_of_two() {
        return Err("spill partition count must be a power of two in 1..=1024");
    }
    let mut hash = FNV_OFFSET;
    for &byte in key {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    Ok((hash as usize) & (partitions - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_partition_ids_are_independent_of_process_hash_seeds() {
        assert_eq!(spill_partition_for_key(b"abc", 16), Ok(11));
        assert_eq!(spill_partition_for_key(b"abc", 16), Ok(11));
        assert_eq!(spill_partition_for_key(b"other", 1), Ok(0));
    }

    #[test]
    fn invalid_partition_counts_fail_closed() {
        for count in [0, 3, 1025, usize::MAX] {
            assert!(spill_partition_for_key(b"key", count).is_err());
        }
    }
}
