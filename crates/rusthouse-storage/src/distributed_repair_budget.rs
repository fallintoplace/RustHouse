//! Bounded under-replication repair planning without network side effects.
use std::collections::HashSet;

const MAX_REPAIR_JOBS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairJob {
    pub shard: String,
    pub bytes: u64,
    pub missing_replicas: u8,
}

fn valid_shard(shard: &str) -> bool {
    !shard.is_empty()
        && shard.len() <= 128
        && shard.as_bytes()[0].is_ascii_alphanumeric()
        && shard
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_repair_job(job: &RepairJob) -> bool {
    job.missing_replicas != 0 && valid_shard(&job.shard)
}

fn repair_priority(left: &RepairJob, right: &RepairJob) -> std::cmp::Ordering {
    right
        .missing_replicas
        .cmp(&left.missing_replicas)
        .then_with(|| left.bytes.cmp(&right.bytes))
        .then_with(|| left.shard.cmp(&right.shard))
}

fn validate_repair_plan_bounds(jobs: &[RepairJob], job_limit: usize) -> Result<(), &'static str> {
    if jobs.len() > MAX_REPAIR_JOBS || job_limit > MAX_REPAIR_JOBS {
        return Err("repair job count exceeds bound");
    }
    Ok(())
}

fn validate_repair_jobs(jobs: &[RepairJob]) -> Result<(), &'static str> {
    if jobs.len() <= 8 {
        for (index, job) in jobs.iter().enumerate() {
            if !valid_repair_job(job) || jobs[..index].iter().any(|prior| prior.shard == job.shard)
            {
                return Err("invalid or duplicate repair job");
            }
        }
        return Ok(());
    }
    // Repair inventories are often emitted in shard order. Validate them
    // with adjacent comparisons and no hashing or heap allocation.
    if jobs.windows(2).all(|pair| pair[0].shard < pair[1].shard) {
        if jobs.iter().any(|job| !valid_repair_job(job)) {
            return Err("invalid or duplicate repair job");
        }
        return Ok(());
    }
    let mut unique = HashSet::with_capacity(jobs.len());
    for job in jobs {
        if !valid_repair_job(job) || !unique.insert(job.shard.as_str()) {
            return Err("invalid or duplicate repair job");
        }
    }
    Ok(())
}

// Preserve strict priority order and byte-budget skips while cloning only
// the jobs admitted to the current repair batch.
fn select_repairs(ordered: &[&RepairJob], byte_budget: u64, job_limit: usize) -> Vec<RepairJob> {
    let mut selected = Vec::with_capacity(ordered.len().min(job_limit));
    let mut remaining = byte_budget;
    for &job in ordered {
        if selected.len() == job_limit {
            break;
        }
        if job.bytes <= remaining {
            remaining -= job.bytes;
            selected.push(job.clone());
        }
    }
    selected
}

// Work with borrowed jobs until the scheduler knows which ones fit.
// This avoids cloning every pending job when only a small batch is selected.
fn plan_ranked_repairs<'a>(
    jobs: impl Iterator<Item = &'a RepairJob> + Clone,
    byte_budget: u64,
    job_limit: usize,
) -> Vec<RepairJob> {
    if job_limit == 0 {
        return Vec::new();
    }
    let eligible = jobs.filter(|job| job.bytes <= byte_budget);
    if job_limit == 1 {
        return eligible
            .min_by(|left, right| repair_priority(left, right))
            .cloned()
            .into_iter()
            .collect();
    }
    if job_limit == 2 {
        // Select the first priority, then re-scan only jobs affordable after
        // that reservation. A two-job poll needs no sorting or reference Vec.
        let Some(first) = eligible
            .clone()
            .min_by(|left, right| repair_priority(left, right))
        else {
            return Vec::new();
        };
        let remaining = byte_budget - first.bytes;
        let second = eligible
            .filter(|job| job.shard != first.shard && job.bytes <= remaining)
            .min_by(|left, right| repair_priority(left, right));
        let mut selected = Vec::with_capacity(2);
        selected.push(first.clone());
        if let Some(second) = second {
            selected.push(second.clone());
        }
        return selected;
    }
    if job_limit <= 8 {
        // When the entire queue fits the byte budget, priority determines
        // the exact answer. Keep only the best few borrowed jobs rather than
        // allocating a full ranking for thousands of pending repairs.
        let total = eligible
            .clone()
            .try_fold(0u64, |sum, job| sum.checked_add(job.bytes));
        if total.is_some_and(|bytes| bytes <= byte_budget) {
            let mut best: Vec<&RepairJob> = Vec::with_capacity(job_limit);
            for job in eligible {
                let position = best.partition_point(|prior| repair_priority(prior, job).is_lt());
                if position < job_limit {
                    best.insert(position, job);
                    if best.len() > job_limit {
                        best.pop();
                    }
                }
            }
            return best.into_iter().cloned().collect();
        }
    }
    let mut ranked: Vec<&RepairJob> = eligible.collect();
    if job_limit.saturating_mul(4) < ranked.len() {
        // Keep widening only when high-priority repairs exhaust the byte
        // budget before filling the batch. Avoid sorting the full queue.
        let mut prefix = job_limit.saturating_mul(2).min(ranked.len());
        loop {
            if prefix < ranked.len() {
                ranked.select_nth_unstable_by(prefix, |a, b| repair_priority(a, b));
            }
            ranked[..prefix].sort_unstable_by(|a, b| repair_priority(a, b));
            let selected = select_repairs(&ranked[..prefix], byte_budget, job_limit);
            if selected.len() == job_limit || prefix == ranked.len() {
                return selected;
            }
            prefix = prefix.saturating_mul(2).min(ranked.len());
        }
    }
    // A fully affordable queue only needs its top job_limit priorities.
    // Partition first, then sort the bounded prefix. Retain the full
    // greedy ordering if the queue exceeds the available byte budget.
    if ranked.len() > job_limit
        && ranked
            .iter()
            .try_fold(0u64, |total, job| total.checked_add(job.bytes))
            .is_some_and(|total| total <= byte_budget)
    {
        ranked.select_nth_unstable_by(job_limit, |left, right| repair_priority(left, right));
        ranked.truncate(job_limit);
    }
    ranked.sort_unstable_by(|left, right| repair_priority(left, right));
    select_repairs(&ranked, byte_budget, job_limit)
}

/// Prioritize severe under-replication, then smaller objects, then shard ID.
/// The result is a schedule only; a transfer engine must execute it.
pub fn plan_repairs(
    jobs: &[RepairJob],
    byte_budget: u64,
    job_limit: usize,
) -> Result<Vec<RepairJob>, &'static str> {
    validate_repair_plan_bounds(jobs, job_limit)?;
    validate_repair_jobs(jobs)?;
    Ok(plan_ranked_repairs(jobs.iter(), byte_budget, job_limit))
}

#[cfg(test)]
mod expanding_repair_selection_tests {
    use super::*;

    #[test]
    fn bounded_repairs_match_the_complete_priority_schedule() {
        let jobs = (0..4096usize)
            .rev()
            .map(|index| RepairJob {
                shard: format!("shard_{index:04}"),
                bytes: if index % 19 == 0 {
                    0
                } else {
                    ((index * 47) % 1024) as u64
                },
                missing_replicas: ((index % 5) + 1) as u8,
            })
            .collect::<Vec<_>>();
        let mut full = jobs.iter().collect::<Vec<_>>();
        full.sort_unstable_by(|a, b| repair_priority(a, b));
        for budget in [0u64, 10, 200, 1024, 8192, u64::MAX] {
            for limit in [2usize, 3, 8, 17, 63] {
                let mut remaining = budget;
                let expected = full
                    .iter()
                    .filter(|job| {
                        if job.bytes <= remaining {
                            remaining -= job.bytes;
                            true
                        } else {
                            false
                        }
                    })
                    .take(limit)
                    .map(|job| (**job).clone())
                    .collect::<Vec<_>>();
                assert_eq!(plan_repairs(&jobs, budget, limit).unwrap(), expected);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_large_repair_queues_preserve_duplicate_validation() {
        for count in [1usize, 2, 8, 9, 32] {
            let mut jobs = (0..count)
                .map(|index| RepairJob {
                    shard: format!("shard_{index}"),
                    bytes: 1,
                    missing_replicas: 1,
                })
                .collect::<Vec<_>>();
            assert_eq!(plan_repairs(&jobs, 100, count).unwrap().len(), count);
            if count > 1 {
                jobs[count - 1].shard = jobs[0].shard.clone();
                assert!(plan_repairs(&jobs, 100, count).is_err());
            }
        }
    }

    #[test]
    fn schedules_urgent_repair_without_exceeding_budget() {
        let jobs = vec![
            RepairJob {
                shard: "a".into(),
                bytes: 40,
                missing_replicas: 1,
            },
            RepairJob {
                shard: "b".into(),
                bytes: 60,
                missing_replicas: 3,
            },
            RepairJob {
                shard: "c".into(),
                bytes: 20,
                missing_replicas: 2,
            },
        ];
        let plan = plan_repairs(&jobs, 80, 2).unwrap();
        assert_eq!(
            plan.iter()
                .map(|job| job.shard.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c"]
        );
    }

    #[test]
    fn rejects_ambiguous_jobs() {
        let job = RepairJob {
            shard: "a".into(),
            bytes: 1,
            missing_replicas: 1,
        };
        assert!(plan_repairs(&[job.clone(), job], 10, 2).is_err());
    }
    #[test]
    fn skips_unschedulable_jobs_without_losing_lower_priority_work() {
        let jobs = vec![
            RepairJob {
                shard: "urgent".into(),
                bytes: 100,
                missing_replicas: 3,
            },
            RepairJob {
                shard: "small".into(),
                bytes: 1,
                missing_replicas: 2,
            },
            RepairJob {
                shard: "extra".into(),
                bytes: 1,
                missing_replicas: 1,
            },
        ];
        let plan = plan_repairs(&jobs, 1, 1).unwrap();
        assert_eq!(plan[0].shard, "small");
        assert_eq!(plan.len(), 1);
        assert!(plan_repairs(&jobs, 1, 0).unwrap().is_empty());
    }
}

/// Do not schedule repairs for shards already assigned to an active worker.
/// In-flight identifiers are trusted scheduler state, not completion claims.
pub fn plan_repairs_excluding(
    jobs: &[RepairJob],
    in_flight: &[&str],
    byte_budget: u64,
    job_limit: usize,
) -> Result<Vec<RepairJob>, &'static str> {
    validate_repair_plan_bounds(jobs, job_limit)?;
    if in_flight.len() > MAX_REPAIR_JOBS {
        return Err("too many in-flight repair shards");
    }
    validate_repair_jobs(jobs)?;
    // A handful of active repairs is common even when the waiting queue is
    // large. Avoid building a transient hash table for up to four exclusions.
    if in_flight.len() <= 4 {
        for (index, &shard) in in_flight.iter().enumerate() {
            if !valid_shard(shard) || in_flight[..index].contains(&shard) {
                return Err("invalid or duplicate in-flight repair shard");
            }
        }
        return Ok(plan_ranked_repairs(
            jobs.iter()
                .filter(|job| !in_flight.contains(&job.shard.as_str())),
            byte_budget,
            job_limit,
        ));
    }
    let mut excluded = HashSet::with_capacity(in_flight.len());
    for &shard in in_flight {
        if !valid_shard(shard) || !excluded.insert(shard) {
            return Err("invalid or duplicate in-flight repair shard");
        }
    }
    Ok(plan_ranked_repairs(
        jobs.iter()
            .filter(|job| !excluded.contains(job.shard.as_str())),
        byte_budget,
        job_limit,
    ))
}

#[cfg(test)]
mod in_flight_repair_tests {
    use super::*;

    #[test]
    fn borrowed_plan_matches_full_plan_when_nothing_is_in_flight() {
        let jobs = (0..1024)
            .map(|index| RepairJob {
                shard: format!("shard_{index:04}"),
                bytes: 1 + index % 17,
                missing_replicas: 1 + (index % 3) as u8,
            })
            .collect::<Vec<_>>();
        for limit in [0, 1, 2, 8, 100, 1024] {
            for budget in [0, 1, 13, 128, 1024] {
                assert_eq!(
                    plan_repairs_excluding(&jobs, &[], budget, limit).unwrap(),
                    plan_repairs(&jobs, budget, limit).unwrap(),
                );
            }
        }
        let selected = plan_repairs_excluding(&jobs, &["shard_0001"], 256, 3).unwrap();
        assert!(selected.iter().all(|job| job.shard != "shard_0001"));
        assert!(plan_repairs_excluding(&jobs, &[], 100, MAX_REPAIR_JOBS + 1).is_err());
    }

    #[test]
    fn avoids_duplicate_urgent_repairs() {
        let jobs = [
            RepairJob {
                shard: "urgent".into(),
                bytes: 20,
                missing_replicas: 3,
            },
            RepairJob {
                shard: "pending".into(),
                bytes: 10,
                missing_replicas: 2,
            },
        ];
        let selected = plan_repairs_excluding(&jobs, &["urgent"], 20, 2).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].shard, "pending");
        assert!(plan_repairs_excluding(&jobs, &["urgent", "urgent"], 20, 2).is_err());
        assert!(plan_repairs_excluding(&jobs, &["../bad"], 20, 2).is_err());
    }
}

/// Plan repairs with an explicit replication factor. Reject impossible deficits.
pub fn plan_repairs_for_replication_factor(
    jobs: &[RepairJob],
    replication_factor: u8,
    byte_budget: u64,
    job_limit: usize,
) -> Result<Vec<RepairJob>, &'static str> {
    if replication_factor == 0 || replication_factor > 64 {
        return Err("invalid replica count for repair planning");
    }
    if jobs
        .iter()
        .any(|job| job.missing_replicas > replication_factor)
    {
        return Err("repair deficit exceeds replication factor");
    }
    plan_repairs(jobs, byte_budget, job_limit)
}

#[cfg(test)]
mod replication_factor_tests {
    use super::*;

    #[test]
    fn rejects_impossible_replica_deficits_before_scheduling() {
        let jobs = [RepairJob {
            shard: "a".into(),
            bytes: 10,
            missing_replicas: 3,
        }];
        assert!(plan_repairs_for_replication_factor(&jobs, 2, 10, 1).is_err());
        assert!(plan_repairs_for_replication_factor(&jobs, 0, 10, 1).is_err());
        assert!(plan_repairs_for_replication_factor(&jobs, 17, 10, 1).is_err());
        assert_eq!(
            plan_repairs_for_replication_factor(&jobs, 3, 10, 1).unwrap(),
            jobs.to_vec()
        );
    }
}

#[cfg(test)]
mod bounded_repair_prefix_tests {
    use super::*;

    #[test]
    fn fully_affordable_prefix_matches_full_priority_order() {
        let jobs = (0..4096usize)
            .rev()
            .map(|index| RepairJob {
                shard: format!("shard_{index:05}"),
                bytes: (index % 11) as u64,
                missing_replicas: (1 + (index % 16)) as u8,
            })
            .collect::<Vec<_>>();
        let full = plan_repairs(&jobs, u64::MAX, jobs.len()).unwrap();
        for limit in [2, 3, 8, 64, 512, 2048, 4096] {
            assert_eq!(
                plan_repairs(&jobs, u64::MAX, limit).unwrap(),
                full.iter().take(limit).cloned().collect::<Vec<_>>()
            );
        }
        let excluded = ["shard_00001", "shard_04095"];
        let entire = plan_repairs_excluding(&jobs, &excluded, u64::MAX, jobs.len()).unwrap();
        for limit in [2, 8, 128] {
            assert_eq!(
                plan_repairs_excluding(&jobs, &excluded, u64::MAX, limit).unwrap(),
                entire.iter().take(limit).cloned().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn constrained_budget_still_skips_large_urgent_repairs() {
        let jobs = [
            RepairJob {
                shard: "urgent".into(),
                bytes: 20,
                missing_replicas: 16,
            },
            RepairJob {
                shard: "zero".into(),
                bytes: 0,
                missing_replicas: 10,
            },
            RepairJob {
                shard: "second".into(),
                bytes: 3,
                missing_replicas: 5,
            },
            RepairJob {
                shard: "third".into(),
                bytes: 4,
                missing_replicas: 3,
            },
        ];
        let chosen = plan_repairs(&jobs, 3, 2).unwrap();
        assert_eq!(
            chosen.iter().map(|j| j.shard.as_str()).collect::<Vec<_>>(),
            vec!["zero", "second"]
        );
        assert_eq!(plan_repairs(&jobs, 0, 2).unwrap()[0].shard, "zero");
        assert!(plan_repairs(&jobs, 0, 2).unwrap().len() == 1);
    }
}

/**
 * One missing target replica for a shard. Multiple targets may legitimately
 * share the same shard ID, but the target pair must be unique.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetedRepairJob {
    pub shard: String,
    pub target: String,
    pub bytes: u64,
    pub missing_replicas: u8,
}

// Apply the greedy byte and shard caps to an exact priority prefix.
// Each expansion starts from the original budgets, so skipped jobs cannot
// leak reservations into the next candidate prefix.
fn choose_targeted_repairs(
    ranked: &[&TargetedRepairJob],
    baseline_capacity: &std::collections::HashMap<&str, u64>,
    total_byte_budget: u64,
    job_limit: usize,
) -> Vec<TargetedRepairJob> {
    use std::collections::HashMap;
    let mut capacity = baseline_capacity.clone();
    let mut selected = Vec::with_capacity(job_limit.min(ranked.len()));
    let mut scheduled_by_shard = HashMap::<&str, u8>::new();
    let mut remaining_total = total_byte_budget;
    for &job in ranked {
        if selected.len() >= job_limit {
            break;
        }
        let scheduled = scheduled_by_shard.entry(job.shard.as_str()).or_default();
        if *scheduled >= job.missing_replicas || job.bytes > remaining_total {
            continue;
        }
        let remaining_node = capacity
            .get_mut(job.target.as_str())
            .expect("all targeted repair destinations were validated");
        if job.bytes > *remaining_node {
            continue;
        }
        *remaining_node -= job.bytes;
        remaining_total -= job.bytes;
        *scheduled += 1;
        selected.push(job.clone());
    }
    selected
}

/// Build a deterministic repair batch that respects both the global transfer
/// budget and each target's remaining disk capacity. Never schedule more
/// targets for a shard than its declared replication deficit.
///
/// This only plans transfers. Capacity figures must be checked again at
/// execution time; returned jobs do not reserve any physical storage.
pub fn plan_targeted_repairs(
    jobs: &[TargetedRepairJob],
    target_capacity: &[(&str, u64)],
    total_byte_budget: u64,
    job_limit: usize,
) -> Result<Vec<TargetedRepairJob>, &'static str> {
    use std::collections::HashMap;
    if jobs.len() > MAX_REPAIR_JOBS
        || job_limit > MAX_REPAIR_JOBS
        || target_capacity.is_empty()
        || target_capacity.len() > 64
    {
        return Err("invalid targeted repair scheduler bounds");
    }
    let valid_node = |node: &str| {
        !node.is_empty()
            && node.len() <= 64
            && node
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    };
    let mut capacity = HashMap::with_capacity(target_capacity.len());
    for &(node, bytes) in target_capacity {
        if !valid_node(node) || capacity.insert(node, bytes).is_some() {
            return Err("invalid or duplicate repair destination");
        }
    }
    // Common small repair polls need neither a shard map nor a pair set.
    // All input validation still completes before scheduling any repair.
    let mut pairs = (jobs.len() > 8).then(|| HashSet::with_capacity(jobs.len()));
    let mut shard_state = (jobs.len() > 8).then(HashMap::new);
    for (index, job) in jobs.iter().enumerate() {
        let duplicate_pair = if let Some(seen) = pairs.as_mut() {
            !seen.insert((job.shard.as_str(), job.target.as_str()))
        } else {
            jobs[..index]
                .iter()
                .any(|previous| previous.shard == job.shard && previous.target == job.target)
        };
        if !valid_shard(&job.shard)
            || !valid_node(&job.target)
            || !(1..=64).contains(&job.missing_replicas)
            || !capacity.contains_key(job.target.as_str())
            || duplicate_pair
        {
            return Err("invalid or duplicate targeted repair job");
        }
        let state = (job.bytes, job.missing_replicas);
        let inconsistent = if let Some(seen) = shard_state.as_mut() {
            seen.insert(job.shard.as_str(), state)
                .is_some_and(|previous| previous != state)
        } else {
            jobs[..index].iter().any(|previous| {
                previous.shard == job.shard && (previous.bytes, previous.missing_replicas) != state
            })
        };
        if inconsistent {
            return Err("inconsistent repair shard size or deficit");
        }
    }
    let priority = |left: &&TargetedRepairJob, right: &&TargetedRepairJob| {
        right
            .missing_replicas
            .cmp(&left.missing_replicas)
            .then_with(|| left.bytes.cmp(&right.bytes))
            .then_with(|| left.shard.cmp(&right.shard))
            .then_with(|| left.target.cmp(&right.target))
    };
    if job_limit == 0 {
        return Ok(Vec::new());
    }
    // An initially unaffordable job never becomes affordable while either
    // budget only decreases. Exclude it before selecting and ranking.
    let eligible = jobs.iter().filter(|job| {
        job.bytes <= total_byte_budget
            && capacity
                .get(job.target.as_str())
                .is_some_and(|&available| job.bytes <= available)
    });
    if job_limit == 1 {
        // Most repair polls request one operation. Find its winner in one
        // scan without a 4096-entry sort or candidate vector.
        return Ok(eligible.min_by(priority).cloned().into_iter().collect());
    }
    let mut ranked = eligible.collect::<Vec<_>>();
    if job_limit.saturating_mul(4) < ranked.len() {
        // Expand the exact priority prefix only when a shard deficit or
        // destination budget prevents it from filling the requested batch.
        // Earlier prefixes are fully sorted and re-evaluated on expansion.
        let mut prefix = job_limit.saturating_mul(2).min(ranked.len());
        loop {
            if prefix < ranked.len() {
                ranked.select_nth_unstable_by(prefix, priority);
            }
            ranked[..prefix].sort_unstable_by(priority);
            let selected =
                choose_targeted_repairs(&ranked[..prefix], &capacity, total_byte_budget, job_limit);
            if selected.len() == job_limit || prefix == ranked.len() {
                return Ok(selected);
            }
            prefix = prefix.saturating_mul(2).min(ranked.len());
        }
    }
    ranked.sort_unstable_by(priority);
    Ok(choose_targeted_repairs(
        &ranked,
        &capacity,
        total_byte_budget,
        job_limit,
    ))
}

#[cfg(test)]
mod targeted_repair_tests {
    use super::*;

    fn job(shard: &str, target: &str, bytes: u64, missing: u8) -> TargetedRepairJob {
        TargetedRepairJob {
            shard: shard.into(),
            target: target.into(),
            bytes,
            missing_replicas: missing,
        }
    }

    #[test]
    fn respects_global_and_per_node_budgets_without_overrepair() {
        let jobs = [
            job("urgent", "a", 60, 2),
            job("urgent", "b", 60, 2),
            job("urgent", "c", 60, 2),
            job("less_urgent", "c", 20, 1),
            job("less_urgent", "a", 20, 1),
        ];
        let result =
            plan_targeted_repairs(&jobs, &[("a", 10), ("b", 100), ("c", 50)], 80, 5).unwrap();
        assert_eq!(
            result,
            vec![job("urgent", "b", 60, 2), job("less_urgent", "c", 20, 1),]
        );
        let consistent = [job("x", "a", 1, 1), job("x", "b", 1, 1)];
        assert_eq!(
            plan_targeted_repairs(&consistent, &[("a", 10), ("b", 10)], 10, 2,)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn all_inventories_and_reordering_produce_stable_priorities() {
        let jobs = (0..250)
            .map(|i| {
                job(
                    &format!("shard_{}", i / 3),
                    &format!("node_{}", i % 3),
                    1 + (i / 3) as u64 % 31,
                    3,
                )
            })
            .collect::<Vec<_>>();
        let targets = [("node_0", 500), ("node_1", 500), ("node_2", 500)];
        let forward = plan_targeted_repairs(&jobs, &targets, 1000, 80).unwrap();
        let mut reversed = jobs.clone();
        reversed.reverse();
        assert_eq!(
            plan_targeted_repairs(&reversed, &targets, 1000, 80).unwrap(),
            forward,
        );
        assert!(forward.len() <= 80);
        let total = forward.iter().map(|job| job.bytes).sum::<u64>();
        assert!(total <= 1000);
    }

    #[test]
    fn validation_remains_strict_even_with_zero_budget() {
        let destinations = [("a", 100), ("b", 100)];
        assert!(
            plan_targeted_repairs(
                &[job("x", "a", 1, 1), job("x", "a", 1, 1)],
                &destinations,
                0,
                0,
            )
            .is_err()
        );
        assert!(
            plan_targeted_repairs(
                &[job("x", "a", 1, 1), job("x", "b", 2, 1)],
                &destinations,
                0,
                0,
            )
            .is_err()
        );
        assert!(plan_targeted_repairs(&[job("x", "unknown", 1, 1)], &destinations, 0, 0,).is_err());
        assert!(plan_targeted_repairs(&[job("x", "a", 1, 0)], &destinations, 0, 0,).is_err());
        assert!(plan_targeted_repairs(&[], &[("a", 1), ("a", 2)], 0, 0,).is_err());
        assert!(plan_targeted_repairs(&[], &[], 0, 0,).is_err());
        assert!(
            plan_targeted_repairs(&[job("x", "a", 1, 1)], &destinations, u64::MAX, 0,)
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod bounded_targeted_repair_priority_tests {
    use super::*;

    fn reference(
        jobs: &[TargetedRepairJob],
        targets: &[(&str, u64)],
        total: u64,
        limit: usize,
    ) -> Vec<TargetedRepairJob> {
        use std::collections::HashMap;
        let mut ranked = jobs.to_vec();
        ranked.sort_unstable_by(|left, right| {
            right
                .missing_replicas
                .cmp(&left.missing_replicas)
                .then_with(|| left.bytes.cmp(&right.bytes))
                .then_with(|| left.shard.cmp(&right.shard))
                .then_with(|| left.target.cmp(&right.target))
        });
        let mut available = targets.iter().copied().collect::<HashMap<_, _>>();
        let mut used = HashMap::<&str, u8>::new();
        let mut budget = total;
        let mut result = Vec::new();
        for job in &ranked {
            if result.len() == limit {
                break;
            }
            let count = used.entry(job.shard.as_str()).or_default();
            let node = available.get_mut(job.target.as_str()).unwrap();
            if *count < job.missing_replicas && job.bytes <= budget && job.bytes <= *node {
                *count += 1;
                budget -= job.bytes;
                *node -= job.bytes;
                result.push(job.clone());
            }
        }
        result
    }

    #[test]
    fn bounded_prefix_matches_full_global_sort_across_skips_and_shard_caps() {
        let jobs = (0..2048usize)
            .map(|index| {
                let shard = index / 4;
                TargetedRepairJob {
                    shard: format!("shard_{shard}"),
                    target: format!("node_{}", index % 4),
                    bytes: if shard % 13 == 0 {
                        0
                    } else {
                        (shard % 53 + 1) as u64
                    },
                    missing_replicas: (shard % 3 + 1) as u8,
                }
            })
            .collect::<Vec<_>>();
        let targets = [
            ("node_0", 1350),
            ("node_1", 1900),
            ("node_2", 440),
            ("node_3", 2000),
        ];
        for budget in [0, 1, 42, 101, 3_000, 100_000] {
            for limit in [0, 1, 2, 3, 7, 13, 64, 1024] {
                let expected = reference(&jobs, &targets, budget, limit);
                let actual = plan_targeted_repairs(&jobs, &targets, budget, limit).unwrap();
                assert_eq!(actual, expected, "budget={budget} limit={limit}");
                let mut reversed = jobs.clone();
                reversed.reverse();
                assert_eq!(
                    plan_targeted_repairs(&reversed, &targets, budget, limit).unwrap(),
                    expected,
                    "reversed budget={budget} limit={limit}"
                );
            }
        }
    }
}

#[cfg(test)]
mod small_in_flight_batch_tests {
    use super::*;

    #[test]
    fn small_exclusions_match_hash_planner_across_large_queues() {
        let jobs = (0..4096usize)
            .map(|i| RepairJob {
                shard: format!("shard_{i:04}"),
                bytes: 1 + (i % 17) as u64,
                missing_replicas: 1 + (i % 5) as u8,
            })
            .collect::<Vec<_>>();
        for exclusions in [
            vec![],
            vec!["shard_0001"],
            vec!["shard_0001", "shard_0200", "shard_1000", "shard_4095"],
        ] {
            let active = exclusions.iter().copied().collect::<HashSet<_>>();
            let expected = plan_ranked_repairs(
                jobs.iter()
                    .filter(|job| !active.contains(job.shard.as_str())),
                2048,
                32,
            );
            assert_eq!(
                plan_repairs_excluding(&jobs, &exclusions, 2048, 32).unwrap(),
                expected,
            );
        }
        assert!(plan_repairs_excluding(&jobs, &["shard_1", "shard_1"], 100, 3).is_err());
        assert!(plan_repairs_excluding(&jobs, &["shard_1", "../invalid"], 100, 3).is_err());
    }
}

#[cfg(test)]
mod linear_two_repair_selection_tests {
    use super::*;

    fn full_sort(jobs: &[RepairJob], byte_budget: u64) -> Vec<RepairJob> {
        let mut ranked = jobs.iter().collect::<Vec<_>>();
        ranked.sort_unstable_by(|left, right| repair_priority(left, right));
        let mut remaining = byte_budget;
        let mut selected = Vec::new();
        for job in ranked {
            if job.bytes <= remaining {
                remaining -= job.bytes;
                selected.push(job.clone());
                if selected.len() == 2 {
                    break;
                }
            }
        }
        selected
    }

    #[test]
    fn exactly_two_picks_match_full_priority_order_for_skewed_budgets() {
        let jobs = (0..2048)
            .map(|index| RepairJob {
                shard: format!("shard_{index:04}"),
                bytes: if index % 11 == 0 {
                    0
                } else {
                    (index * 131 % 2047) as u64
                },
                missing_replicas: (1 + index % 9) as u8,
            })
            .collect::<Vec<_>>();
        for budget in [0u64, 1, 7, 30, 500, 2047, 10_000, u64::MAX] {
            assert_eq!(
                plan_repairs(&jobs, budget, 2).unwrap(),
                full_sort(&jobs, budget)
            );
            let excluded = ["shard_0000", "shard_0001", "shard_0002"];
            let filtered = jobs
                .iter()
                .filter(|job| !excluded.contains(&job.shard.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                plan_repairs_excluding(&jobs, &excluded, budget, 2).unwrap(),
                full_sort(&filtered, budget),
            );
        }
    }

    #[test]
    fn larger_candidate_can_leave_no_budget_for_second_repair() {
        let jobs = [
            RepairJob {
                shard: "first".into(),
                bytes: 100,
                missing_replicas: 3,
            },
            RepairJob {
                shard: "cannot_fit".into(),
                bytes: 5,
                missing_replicas: 2,
            },
            RepairJob {
                shard: "zero".into(),
                bytes: 0,
                missing_replicas: 1,
            },
        ];
        assert_eq!(
            plan_repairs(&jobs, 100, 2)
                .unwrap()
                .iter()
                .map(|job| job.shard.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "zero"],
        );
    }
}

#[cfg(test)]
mod canonical_repair_shard_tests {
    use super::*;

    #[test]
    fn repair_shards_match_the_object_key_and_placement_prefix_rules() {
        for shard in ["-not-a-key", "_not-a-key", ""] {
            let jobs = [RepairJob {
                shard: shard.into(),
                bytes: 1,
                missing_replicas: 1,
            }];
            assert!(plan_repairs(&jobs, 5, 1).is_err(), "shard={shard:?}");
            assert!(
                plan_repairs_excluding(&[], &[shard], 5, 1).is_err(),
                "shard={shard:?}"
            );
        }
        for shard in ["a", "0_worker", "shard-2"] {
            let jobs = [RepairJob {
                shard: shard.into(),
                bytes: 1,
                missing_replicas: 1,
            }];
            assert_eq!(plan_repairs(&jobs, 5, 1).unwrap(), jobs);
        }
    }
}

#[cfg(test)]
mod small_targeted_repair_validation_tests {
    use super::*;

    fn job(shard: &str, target: &str, bytes: u64, missing: u8) -> TargetedRepairJob {
        TargetedRepairJob {
            shard: shard.into(),
            target: target.into(),
            bytes,
            missing_replicas: missing,
        }
    }

    #[test]
    fn selects_small_batches_and_skips_unaffordable_targets() {
        let jobs = [
            job("a", "n1", 5, 2),
            job("a", "n2", 5, 2),
            job("b", "n1", 1, 1),
        ];
        let chosen = plan_targeted_repairs(&jobs, &[("n1", 5), ("n2", 5)], 10, 3).unwrap();
        assert_eq!(chosen, vec![jobs[0].clone(), jobs[1].clone()]);
    }

    #[test]
    fn detects_duplicate_pairs_and_inconsistent_shards_before_zero_budget() {
        let first = job("a", "n1", 3, 2);
        let same_pair = job("a", "n1", 3, 2);
        let wrong_size = job("a", "n2", 4, 2);
        let targets = [("n1", 10), ("n2", 10)];
        assert!(plan_targeted_repairs(&[first.clone(), same_pair], &targets, 0, 0).is_err());
        assert!(plan_targeted_repairs(&[first, wrong_size], &targets, 0, 0).is_err());
    }

    #[test]
    fn threshold_transition_keeps_strict_validation() {
        let mut jobs = (0..8)
            .map(|i| job(&format!("s{i}"), "n1", 1, 1))
            .collect::<Vec<_>>();
        let targets = [("n1", 16)];
        assert_eq!(
            plan_targeted_repairs(&jobs, &targets, 16, 8).unwrap().len(),
            8
        );
        jobs.push(job("s0", "n1", 1, 1));
        assert!(plan_targeted_repairs(&jobs, &targets, 16, 8).is_err());
    }
}

#[cfg(test)]
mod inline_small_repair_queue_reference_tests {
    use super::*;

    fn jobs(count: usize) -> Vec<RepairJob> {
        (0..count)
            .map(|index| RepairJob {
                shard: format!("shard_{index:05}"),
                bytes: ((index * 37) % 97) as u64,
                missing_replicas: (1 + ((index * 11) % 6)) as u8,
            })
            .collect()
    }

    fn reference(jobs: &[RepairJob], budget: u64, limit: usize) -> Vec<RepairJob> {
        let mut ranked = jobs.to_vec();
        ranked.sort_unstable_by(repair_priority);
        let mut remaining = budget;
        let mut selected = Vec::new();
        for job in ranked {
            if selected.len() == limit {
                break;
            }
            if job.bytes <= remaining {
                remaining -= job.bytes;
                selected.push(job);
            }
        }
        selected
    }

    #[test]
    fn all_affordable_large_queues_match_full_priority_sort() {
        for count in [0, 1, 2, 3, 8, 9, 100, 4096] {
            let jobs = jobs(count);
            for limit in [3, 4, 5, 8] {
                assert_eq!(
                    plan_repairs(&jobs, u64::MAX, limit).unwrap(),
                    reference(&jobs, u64::MAX, limit),
                    "count={count} limit={limit}"
                );
            }
        }
    }

    #[test]
    fn constrained_budget_still_preserves_greedy_priority_skips() {
        let jobs = jobs(600);
        for budget in [0, 1, 50, 500, 10_000] {
            for limit in [3, 4, 8] {
                assert_eq!(
                    plan_repairs(&jobs, budget, limit).unwrap(),
                    reference(&jobs, budget, limit),
                    "budget={budget} limit={limit}"
                );
            }
        }
    }
}

#[cfg(test)]
mod wide_replication_repair_planning_tests {
    use super::*;

    #[test]
    fn plans_a_full_64_target_repair_without_truncating_deficits() {
        let destinations = (0..64).map(|i| format!("node{i}")).collect::<Vec<_>>();
        let capacity = destinations.iter().map(|node| (node.as_str(), 1u64)).collect::<Vec<_>>();
        let jobs = destinations.iter().map(|target| TargetedRepairJob {
            shard: "under_replicated".into(),
            target: target.clone(),
            bytes: 1,
            missing_replicas: 64,
        }).collect::<Vec<_>>();
        let planned = plan_targeted_repairs(&jobs, &capacity, 64, 64).unwrap();
        assert_eq!(planned.len(), 64);
        assert_eq!(planned.iter().map(|job| job.bytes).sum::<u64>(), 64);
        assert!(planned.iter().all(|job| job.missing_replicas == 64));

        let aggregate = [RepairJob { shard: "under_replicated".into(), bytes: 64, missing_replicas: 64 }];
        assert_eq!(plan_repairs_for_replication_factor(&aggregate, 64, 64, 1).unwrap(), aggregate.to_vec());
        assert!(plan_repairs_for_replication_factor(&aggregate, 63, 64, 1).is_err());
        let mut invalid = jobs;
        invalid[0].missing_replicas = 65;
        assert!(plan_targeted_repairs(&invalid, &capacity, 64, 64).is_err());
    }
}

#[cfg(test)]
mod ordered_repair_validation_tests {
    use super::*;

    #[test]
    fn ordered_and_unordered_repair_inventories_have_identical_schedules() {
        let ordered = (0..4096)
            .map(|i| RepairJob {
                shard: format!("shard_{i:04}"),
                bytes: (i % 29) as u64,
                missing_replicas: (i % 5 + 1) as u8,
            })
            .collect::<Vec<_>>();
        let mut unordered = ordered.clone();
        unordered.reverse();
        for budget in [0u64, 11, 257, 8192, u64::MAX] {
            for limit in [0, 1, 2, 8, 64] {
                assert_eq!(
                    plan_repairs(&ordered, budget, limit).unwrap(),
                    plan_repairs(&unordered, budget, limit).unwrap()
                );
            }
        }
        let mut duplicate = ordered.clone();
        duplicate[2048].shard = duplicate[1024].shard.clone();
        assert!(plan_repairs(&duplicate, 9999, 10).is_err());
        let mut invalid = ordered;
        invalid[2048].shard = "bad/name".into();
        assert!(plan_repairs(&invalid, 9999, 10).is_err());
    }
}
