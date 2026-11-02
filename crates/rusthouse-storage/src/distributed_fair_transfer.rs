//! Deterministic, nonmutating fair scheduling of replica byte ranges.
//! Consumers must verify transfers before persisting returned progress.

use std::collections::HashSet;

const MAX_SHARDS: usize = 4096;
const MAX_QUANTUM: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardProgress {
    pub shard: String,
    pub completed: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferSlice {
    pub shard: String,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FairTransferBatch {
    pub slices: Vec<TransferSlice>,
    pub next_progress: Vec<ShardProgress>,
    pub next_cursor: usize,
}

fn valid_shard(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Plan bounded round-robin byte ranges across independent shards.
///
/// A shard receives at most `quantum` bytes per turn. The scheduler advances
/// the next cursor after each slice, so small transfers cannot be starved by
/// a continuously large shard. No input state is mutated; only persist the
/// returned `next_progress` after all planned ranges pass verification.
pub fn plan_fair_transfers(
    progress: &[ShardProgress],
    start_cursor: usize,
    quantum: u64,
    byte_budget: u64,
    max_slices: usize,
) -> Result<FairTransferBatch, &'static str> {
    if progress.is_empty()
        || progress.len() > MAX_SHARDS
        || start_cursor >= progress.len()
        || quantum == 0
        || quantum > MAX_QUANTUM
        || max_slices > MAX_SHARDS
    {
        return Err("invalid fair transfer scheduler limits");
    }
    let mut seen = HashSet::with_capacity(progress.len());
    for item in progress {
        if !valid_shard(&item.shard)
            || item.completed > item.total
            || !seen.insert(item.shard.as_str())
        {
            return Err("invalid or duplicate shard transfer progress");
        }
    }
    let mut next_progress = progress.to_vec();
    let mut slices = Vec::with_capacity(max_slices);
    let mut cursor = start_cursor;
    let mut remaining_budget = byte_budget;
    while remaining_budget > 0 && slices.len() < max_slices {
        let Some(index) = (0..next_progress.len())
            .map(|offset| (cursor + offset) % next_progress.len())
            .find(|&index| next_progress[index].completed < next_progress[index].total)
        else {
            break;
        };
        let item = &mut next_progress[index];
        let available = item.total - item.completed;
        let length = available.min(quantum).min(remaining_budget);
        slices.push(TransferSlice {
            shard: item.shard.clone(),
            offset: item.completed,
            length,
        });
        item.completed += length;
        remaining_budget -= length;
        cursor = (index + 1) % next_progress.len();
    }
    Ok(FairTransferBatch {
        slices,
        next_progress,
        next_cursor: cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(shard: &str, completed: u64, total: u64) -> ShardProgress {
        ShardProgress {
            shard: shard.into(),
            completed,
            total,
        }
    }

    #[test]
    fn long_shard_does_not_starve_short_shards() {
        let jobs = [job("large", 0, 100), job("small", 0, 2), job("middle", 0, 5)];
        let batch = plan_fair_transfers(&jobs, 0, 3, 10, 10).unwrap();
        assert_eq!(
            batch.slices,
            vec![
                TransferSlice { shard: "large".into(), offset: 0, length: 3 },
                TransferSlice { shard: "small".into(), offset: 0, length: 2 },
                TransferSlice { shard: "middle".into(), offset: 0, length: 3 },
                TransferSlice { shard: "large".into(), offset: 3, length: 2 },
            ]
        );
        assert_eq!(batch.next_cursor, 1);
        assert_eq!(jobs[0].completed, 0);
        let followup = plan_fair_transfers(&batch.next_progress, batch.next_cursor, 3, 3, 10).unwrap();
        assert_eq!(followup.slices[0].shard, "middle");
        assert_eq!(followup.slices[0].offset, 3);
    }

    #[test]
    fn every_byte_is_scheduled_exactly_once_across_rounds() {
        let mut jobs = vec![job("a", 0, 17), job("b", 0, 3), job("c", 0, 21)];
        let mut cursor = 2;
        let mut ranges = Vec::new();
        while jobs.iter().any(|job| job.completed < job.total) {
            let batch = plan_fair_transfers(&jobs, cursor, 4, 5, 3).unwrap();
            assert!(!batch.slices.is_empty());
            assert!(batch.slices.iter().map(|slice| slice.length).sum::<u64>() <= 5);
            ranges.extend(batch.slices);
            jobs = batch.next_progress;
            cursor = batch.next_cursor;
        }
        for shard in ["a", "b", "c"] {
            let mut ranges = ranges
                .iter()
                .filter(|slice| slice.shard == shard)
                .collect::<Vec<_>>();
            ranges.sort_by_key(|slice| slice.offset);
            let mut end = 0;
            for slice in ranges {
                assert_eq!(slice.offset, end);
                end += slice.length;
            }
            assert_eq!(end, if shard == "a" { 17 } else if shard == "b" { 3 } else { 21 });
        }
    }

    #[test]
    fn rejects_bad_progress_before_any_scheduling() {
        assert!(plan_fair_transfers(&[], 0, 4, 5, 1).is_err());
        assert!(plan_fair_transfers(&[job("a", 4, 3)], 0, 4, 5, 1).is_err());
        assert!(plan_fair_transfers(&[job("a", 0, 3), job("a", 0, 3)], 0, 4, 5, 1).is_err());
        assert!(plan_fair_transfers(&[job("../bad", 0, 3)], 0, 4, 5, 1).is_err());
        assert!(plan_fair_transfers(&[job("a", 0, 3)], 1, 4, 5, 1).is_err());
        assert!(plan_fair_transfers(&[job("a", 0, 3)], 0, 0, 5, 1).is_err());
    }

    #[test]
    fn zero_budget_does_not_advance_state() {
        let jobs = [job("a", 2, 5), job("b", 1, 5)];
        let batch = plan_fair_transfers(&jobs, 1, 2, 0, 10).unwrap();
        assert!(batch.slices.is_empty());
        assert_eq!(batch.next_progress, jobs);
        assert_eq!(batch.next_cursor, 1);
    }
}
