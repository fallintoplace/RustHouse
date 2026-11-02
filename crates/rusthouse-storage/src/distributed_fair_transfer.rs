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

/// Rebuild progress from externally verified transfer slices.
///
/// A caller must authenticate and check the bytes before supplying each slice.
/// This function only validates that acknowledged ranges are contiguous, do
/// not exceed source limits, and belong to known shards. Failed replay does
/// not change the original progress checkpoint.
pub fn replay_verified_transfer_slices(
    progress: &[ShardProgress],
    verified_slices: &[TransferSlice],
) -> Result<Vec<ShardProgress>, &'static str> {
    use std::collections::HashMap;

    if progress.is_empty()
        || progress.len() > MAX_SHARDS
        || verified_slices.len() > MAX_SHARDS
    {
        return Err("invalid verified transfer replay limits");
    }
    let mut by_name = HashMap::with_capacity(progress.len());
    for (index, item) in progress.iter().enumerate() {
        if !valid_shard(&item.shard)
            || item.completed > item.total
            || by_name.insert(item.shard.as_str(), index).is_some()
        {
            return Err("invalid or duplicate shard transfer progress");
        }
    }
    let mut next = progress.to_vec();
    for slice in verified_slices {
        let Some(&index) = by_name.get(slice.shard.as_str()) else {
            return Err("verified transfer refers to unknown shard");
        };
        let item = &mut next[index];
        if slice.length == 0
            || slice.length > MAX_QUANTUM
            || slice.offset != item.completed
            || slice.length > item.total - item.completed
        {
            return Err("verified transfer range is inconsistent with checkpoint");
        }
        item.completed += slice.length;
    }
    Ok(next)
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
    fn verified_slices_replay_exactly_and_reject_forged_progress() {
        let original = [job("large", 0, 10), job("small", 2, 5)];
        let batch = plan_fair_transfers(&original, 0, 4, 8, 3).unwrap();
        let replayed = replay_verified_transfer_slices(&original, &batch.slices).unwrap();
        assert_eq!(replayed, batch.next_progress);
        assert_eq!(original[0].completed, 0);
        assert_eq!(original[1].completed, 2);

        let mut forged = batch.slices.clone();
        forged[0].offset += 1;
        assert!(replay_verified_transfer_slices(&original, &forged).is_err());

        let mut forged = batch.slices.clone();
        forged[0].length = 11;
        assert!(replay_verified_transfer_slices(&original, &forged).is_err());

        let mut forged = batch.slices.clone();
        forged[0].shard = "other".into();
        assert!(replay_verified_transfer_slices(&original, &forged).is_err());

        let duplicated = [job("same", 0, 1), job("same", 0, 1)];
        assert!(replay_verified_transfer_slices(&duplicated, &[]).is_err());
        assert!(replay_verified_transfer_slices(&original, &[]).is_ok());
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


/// Durable state for weighted deficit-round-robin byte scheduling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedShardProgress {
    pub shard: String,
    pub completed: u64,
    pub total: u64,
    pub weight: u16,
    /// Unused granted byte credits, carried between batches.
    pub deficit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedTransferBatch {
    pub slices: Vec<TransferSlice>,
    pub next_progress: Vec<WeightedShardProgress>,
    pub next_cursor: usize,
}

/// Schedule proportional replica bandwidth without starving small shards.
///
/// Every visited shard receives `quantum * weight` credits before transferring.
/// Remaining credits survive a partial batch; only persist the returned state
/// after the corresponding bytes have been verified. No input is modified.
pub fn plan_weighted_transfers(
    progress: &[WeightedShardProgress],
    start_cursor: usize,
    quantum: u64,
    byte_budget: u64,
    max_slices: usize,
) -> Result<WeightedTransferBatch, &'static str> {
    const MAX_WEIGHT: u16 = 64;
    const MAX_CREDIT: u64 = MAX_QUANTUM * MAX_WEIGHT as u64;
    if progress.is_empty()
        || progress.len() > MAX_SHARDS
        || start_cursor >= progress.len()
        || quantum == 0
        || quantum > MAX_QUANTUM
        || max_slices > MAX_SHARDS
    {
        return Err("invalid weighted transfer limits");
    }
    let mut seen = HashSet::with_capacity(progress.len());
    for item in progress {
        if !valid_shard(&item.shard)
            || item.completed > item.total
            || item.weight == 0
            || item.weight > MAX_WEIGHT
            || item.deficit > MAX_CREDIT
            || !seen.insert(item.shard.as_str())
        {
            return Err("invalid weighted shard progress");
        }
    }
    let mut next_progress = progress.to_vec();
    let mut slices = Vec::with_capacity(max_slices);
    let mut cursor = start_cursor;
    let mut remaining = byte_budget;
    while remaining > 0 && slices.len() < max_slices {
        let Some(index) = (0..next_progress.len())
            .map(|offset| (cursor + offset) % next_progress.len())
            .find(|&index| next_progress[index].completed < next_progress[index].total)
        else {
            break;
        };
        let item = &mut next_progress[index];
        let credit = quantum * u64::from(item.weight);
        item.deficit = item.deficit.saturating_add(credit).min(MAX_CREDIT);
        let length = item
            .deficit
            .min(item.total - item.completed)
            .min(remaining)
            // The receipt replay path accepts at most one quantum per slice.
            .min(MAX_QUANTUM);
        // Both quantum and weight are positive; an unfinished shard can
        // always make progress until the byte budget is exhausted.
        debug_assert!(length > 0);
        slices.push(TransferSlice {
            shard: item.shard.clone(),
            offset: item.completed,
            length,
        });
        item.completed += length;
        item.deficit -= length;
        if item.completed == item.total {
            item.deficit = 0;
        }
        remaining -= length;
        // A high-weight shard may need several replayable slices to spend
        // its earned credit. Keep its turn until less than one full slice
        // remains, so capping frame size does not erase weighted fairness.
        cursor = if item.completed < item.total && item.deficit >= MAX_QUANTUM {
            index
        } else {
            (index + 1) % next_progress.len()
        };
    }
    Ok(WeightedTransferBatch {
        slices,
        next_progress,
        next_cursor: cursor,
    })
}

#[cfg(test)]
mod weighted_transfer_tests {
    use super::*;

    fn job(name: &str, weight: u16, total: u64) -> WeightedShardProgress {
        WeightedShardProgress {
            shard: name.into(),
            completed: 0,
            total,
            weight,
            deficit: 0,
        }
    }

    #[test]
    fn proportional_weights_and_resume_preserve_exact_byte_ranges() {
        let mut progress = vec![job("slow", 1, 100), job("fast", 3, 100)];
        let mut cursor = 0;
        let mut slices = Vec::new();
        for _ in 0..5 {
            let plan = plan_weighted_transfers(&progress, cursor, 2, 16, 4).unwrap();
            assert_eq!(plan.slices.iter().map(|s| s.length).sum::<u64>(), 16);
            slices.extend(plan.slices);
            progress = plan.next_progress;
            cursor = plan.next_cursor;
        }
        assert_eq!(progress[0].completed, 20);
        assert_eq!(progress[1].completed, 60);
        for name in ["slow", "fast"] {
            let mut expected_offset = 0;
            for slice in slices.iter().filter(|slice| slice.shard == name) {
                assert_eq!(slice.offset, expected_offset);
                expected_offset += slice.length;
            }
        }
    }

    #[test]
    fn invalid_inputs_and_zero_budget_do_not_change_source() {
        let source = [job("a", 1, 10), job("b", 2, 10)];
        let batch = plan_weighted_transfers(&source, 1, 1, 0, 2).unwrap();
        assert!(batch.slices.is_empty());
        assert_eq!(batch.next_progress, source);
        assert_eq!(batch.next_cursor, 1);
        let mut wrong = source.to_vec();
        wrong[1].shard = "a".into();
        assert!(plan_weighted_transfers(&wrong, 0, 1, 10, 2).is_err());
        wrong[1].shard = "b".into();
        wrong[1].weight = 0;
        assert!(plan_weighted_transfers(&wrong, 0, 1, 10, 2).is_err());
        assert!(plan_weighted_transfers(&source, 0, 0, 10, 2).is_err());
    }

    #[test]
    fn capacity_arithmetic_handles_largest_offsets() {
        let mut item = job("large", 64, u64::MAX);
        item.completed = u64::MAX - 2;
        let batch = plan_weighted_transfers(&[item], 0, MAX_QUANTUM, 100, 2).unwrap();
        assert_eq!(batch.slices[0].offset, u64::MAX - 2);
        assert_eq!(batch.slices[0].length, 2);
        assert_eq!(batch.next_progress[0].completed, u64::MAX);
    }
}

#[cfg(test)]
mod weighted_replay_slice_bounds_tests {
    use super::*;

    #[test]
    fn weighted_plan_emits_slices_accepted_by_verified_replay() {
        let source = [WeightedShardProgress {
            shard: "large".into(),
            completed: 0,
            total: MAX_QUANTUM * 2,
            weight: 64,
            deficit: 0,
        }];
        let plan = plan_weighted_transfers(
            &source, 0, MAX_QUANTUM, MAX_QUANTUM * 2, 2,
        ).unwrap();
        assert_eq!(plan.slices.len(), 2);
        assert!(plan.slices.iter().all(|slice| slice.length <= MAX_QUANTUM));
        let checkpoint = [ShardProgress {
            shard: "large".into(),
            completed: 0,
            total: MAX_QUANTUM * 2,
        }];
        let replayed = replay_verified_transfer_slices(&checkpoint, &plan.slices).unwrap();
        assert_eq!(replayed[0].completed, plan.next_progress[0].completed);
    }

    #[test]
    fn split_slices_preserve_weighted_bandwidth_shares() {
        let source = [
            WeightedShardProgress {
                shard: "fast".into(),
                completed: 0,
                total: MAX_QUANTUM * 100,
                weight: 64,
                deficit: 0,
            },
            WeightedShardProgress {
                shard: "slow".into(),
                completed: 0,
                total: MAX_QUANTUM * 100,
                weight: 1,
                deficit: 0,
            },
        ];
        let batch = plan_weighted_transfers(
            &source, 0, MAX_QUANTUM, MAX_QUANTUM * 65, 65,
        ).unwrap();
        assert_eq!(batch.slices.len(), 65);
        assert_eq!(batch.slices.iter().filter(|s| s.shard == "fast").count(), 64);
        assert_eq!(batch.slices.iter().filter(|s| s.shard == "slow").count(), 1);
        assert!(batch.slices.iter().all(|slice| slice.length <= MAX_QUANTUM));
        let checkpoint = [
            ShardProgress { shard: "fast".into(), completed: 0, total: MAX_QUANTUM * 100 },
            ShardProgress { shard: "slow".into(), completed: 0, total: MAX_QUANTUM * 100 },
        ];
        let replayed = replay_verified_transfer_slices(&checkpoint, &batch.slices).unwrap();
        for (replayed, planned) in replayed.iter().zip(&batch.next_progress) {
            assert_eq!(replayed.completed, planned.completed);
        }
    }
}
