//! Deterministic retry delays for a future replica transfer scheduler.
use std::time::Duration;

/// Cap exponential retries at max_ms; jitter is stable for the supplied seed.
pub fn retry_delay(
    attempt: u32,
    base_ms: u64,
    max_ms: u64,
    seed: u64,
) -> Result<Duration, &'static str> {
    if attempt > 32 || base_ms == 0 || max_ms == 0 || base_ms > max_ms {
        return Err("invalid replica retry configuration");
    }
    let bounded_backoff_ms = base_ms.saturating_mul(1u64 << attempt).min(max_ms);
    let half = bounded_backoff_ms.div_ceil(2);
    // Mix both seed and attempt before reducing to the jitter range.
    // Multiplication alone maps seed zero to zero for every attempt,
    // synchronizing retries across nodes with an unset seed.
    let mut mixed = seed.wrapping_add(u64::from(attempt).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^= mixed >> 31;
    let jitter = mixed % (bounded_backoff_ms - half + 1);
    Ok(Duration::from_millis(half + jitter))
}

/// Return no scheduled retry when it cannot fit within the remaining deadline.
pub fn retry_delay_with_deadline(
    attempt: u32,
    base_ms: u64,
    max_ms: u64,
    seed: u64,
    remaining: Duration,
) -> Result<Option<Duration>, &'static str> {
    let delay = retry_delay(attempt, base_ms, max_ms, seed)?;
    Ok((delay <= remaining).then_some(delay))
}

/// Precompute up to 33 bounded retry delays without exceeding the total budget.
/// This only schedules delays and never starts network transfers.
pub fn plan_retry_delays(
    attempts: u32,
    base_ms: u64,
    max_ms: u64,
    seed: u64,
    budget: Duration,
) -> Result<Vec<Duration>, &'static str> {
    plan_retry_delays_from(0, attempts, base_ms, max_ms, seed, budget)
}

/// Resume a bounded retry schedule at a persisted attempt number.
///
/// The budget applies only to scheduled future delays, not to elapsed
/// retries. A single skipped attempt must never shift the deterministic
/// jitter seed or bypass exponential backoff. No network action occurs.
pub fn plan_retry_delays_from(
    first_attempt: u32,
    attempts: u32,
    base_ms: u64,
    max_ms: u64,
    seed: u64,
    budget: Duration,
) -> Result<Vec<Duration>, &'static str> {
    if first_attempt > 33 || attempts > 33 - first_attempt {
        return Err("retry count exceeds supported attempt range");
    }
    // Validate even an empty plan or a deadline that excludes all retries.
    retry_delay(0, base_ms, max_ms, seed)?;
    if attempts == 0 || budget < Duration::from_millis(base_ms.div_ceil(2)) {
        return Ok(Vec::new());
    }
    let mut elapsed = Duration::ZERO;
    let mut delays = Vec::with_capacity(attempts as usize);
    for attempt in first_attempt..first_attempt + attempts {
        let delay = retry_delay(attempt, base_ms, max_ms, seed)?;
        if delay > budget.saturating_sub(elapsed) {
            break;
        }
        elapsed += delay;
        delays.push(delay);
    }
    Ok(delays)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_exceeds_deadline_even_with_large_attempts() {
        for attempt in 0..=32 {
            let delay = retry_delay(attempt, 100, 5_000, 1234).unwrap();
            assert!(delay.as_millis() > 0);
            assert!(delay.as_millis() <= 5_000);
        }
    }

    #[test]
    fn one_millisecond_backoff_never_becomes_zero() {
        for seed in [0, 1, u64::MAX] {
            assert_eq!(
                retry_delay(0, 1, 1, seed).unwrap(),
                Duration::from_millis(1)
            );
        }
    }

    #[test]
    fn rejects_zero_and_bad_bounds() {
        assert!(retry_delay(0, 0, 100, 1).is_err());
        assert!(retry_delay(0, 100, 10, 1).is_err());
        assert!(retry_delay(33, 1, 10, 1).is_err());
    }
    #[test]
    fn retry_backoff_is_deterministic_at_integer_boundaries() {
        for attempt in 0..=32 {
            for seed in [0, 1, u64::MAX] {
                let delay = retry_delay(attempt, u64::MAX, u64::MAX, seed).unwrap();
                assert!(delay >= Duration::from_millis(1));
                assert!(delay.as_millis() <= u128::from(u64::MAX));
                assert_eq!(
                    delay,
                    retry_delay(attempt, u64::MAX, u64::MAX, seed).unwrap()
                );
            }
        }
    }
}

#[cfg(test)]
mod retry_deadline_tests {
    use super::*;

    #[test]
    fn skips_retries_that_outlive_the_budget() {
        let delay = retry_delay(3, 100, 1_000, 7).unwrap();
        assert_eq!(
            retry_delay_with_deadline(3, 100, 1_000, 7, delay).unwrap(),
            Some(delay)
        );
        assert_eq!(
            retry_delay_with_deadline(3, 100, 1_000, 7, Duration::ZERO).unwrap(),
            None
        );
        assert!(retry_delay_with_deadline(33, 100, 1_000, 7, delay).is_err());
    }
}

#[cfg(test)]
mod total_retry_budget_tests {
    use super::*;

    #[test]
    fn caps_cumulative_retry_delay_and_is_deterministic() {
        let budget = Duration::from_millis(250);
        let scheduled = plan_retry_delays(33, 100, 1000, 2, budget).unwrap();
        assert!(!scheduled.is_empty());
        assert!(scheduled.iter().copied().sum::<Duration>() <= budget);
        assert_eq!(
            scheduled,
            plan_retry_delays(33, 100, 1000, 2, budget).unwrap()
        );
        assert!(plan_retry_delays(34, 1, 10, 0, budget).is_err());
        assert!(plan_retry_delays(0, 0, 10, 0, budget).is_err());
        assert!(
            plan_retry_delays(5, 100, 1000, 1, Duration::ZERO)
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod retry_plan_single_validation_tests {
    use super::*;

    #[test]
    fn planned_first_delay_matches_the_individual_retry_delay() {
        for seed in [0, 1, 42, u64::MAX] {
            let expected = retry_delay(0, 100, 5000, seed).unwrap();
            assert_eq!(
                plan_retry_delays(1, 100, 5000, seed, Duration::MAX).unwrap(),
                vec![expected]
            );
        }
        assert!(plan_retry_delays(0, 0, 5000, 0, Duration::MAX).is_err());
        assert!(
            plan_retry_delays(0, 100, 5000, 0, Duration::MAX)
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod zero_seed_retry_dispersion_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn zero_seed_jitters_independent_attempts_after_backoff_caps() {
        let mut delays = HashSet::new();
        for attempt in 10..=32 {
            let delay = retry_delay(attempt, 100, 1000, 0).unwrap();
            assert!((500..=1000).contains(&(delay.as_millis() as u64)));
            assert_eq!(delay, retry_delay(attempt, 100, 1000, 0).unwrap());
            delays.insert(delay);
        }
        assert!(
            delays.len() >= 10,
            "capped attempts should not all synchronize"
        );
    }

    #[test]
    fn a_zero_attempt_still_uses_the_replica_seed() {
        let mut delays = HashSet::new();
        for seed in 0..128 {
            delays.insert(retry_delay(0, 1000, 1000, seed).unwrap());
        }
        assert!(delays.len() > 40, "seeds should spread initial retries");
    }
}

#[cfg(test)]
mod planned_retry_consistency_tests {
    use super::*;

    #[test]
    fn budgeted_plan_uses_the_same_jitter_as_individual_retries() {
        for seed in [0, 1, 17, u64::MAX] {
            let planned = plan_retry_delays(33, 125, 10_000, seed, Duration::MAX).unwrap();
            assert_eq!(planned.len(), 33);
            for (attempt, delay) in planned.iter().enumerate() {
                assert_eq!(
                    *delay,
                    retry_delay(attempt as u32, 125, 10_000, seed).unwrap(),
                    "seed={seed}, attempt={attempt}"
                );
            }
            let budget = planned[..4].iter().copied().sum::<Duration>();
            let limited = plan_retry_delays(33, 125, 10_000, seed, budget).unwrap();
            assert_eq!(limited, planned[..4]);
        }
    }
}

#[cfg(test)]
mod subminimum_retry_budget_tests {
    use super::*;

    #[test]
    fn early_exit_preserves_configuration_validation() {
        for seed in [0, 17, u64::MAX] {
            assert!(
                plan_retry_delays(33, 100, 1000, seed, Duration::from_millis(49))
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(plan_retry_delays(1, 0, 1000, 0, Duration::ZERO).is_err());
        assert!(plan_retry_delays(34, 100, 1000, 0, Duration::ZERO).is_err());
        let first = retry_delay(0, 100, 1000, 17).unwrap();
        assert_eq!(
            plan_retry_delays(1, 100, 1000, 17, first).unwrap(),
            vec![first]
        );
    }
}

/// A planned retry at an absolute offset from the beginning of a poll.
/// The scheduler does not send requests or claim retry safety.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaRetryEvent {
    pub node: String,
    pub attempt: u32,
    pub due_after: Duration,
}

/// Merge deterministic per-node retry sequences into a time-ordered schedule.
///
/// The total deadline is shared, not spent separately by every node. All
/// configured nodes use the same backoff limits but independent jitter seeds.
/// Results are bounded to 64 nodes x 33 retry attempts.
pub fn plan_replica_retry_timeline(
    node_seeds: &[(&str, u64)],
    attempts_per_node: u32,
    base_ms: u64,
    max_ms: u64,
    deadline: Duration,
) -> Result<Vec<ReplicaRetryEvent>, &'static str> {
    plan_replica_retry_timeline_from(
        node_seeds, 0, attempts_per_node, base_ms, max_ms, deadline,
    )
}

/// Continue a deterministic multi-node retry schedule from an absolute attempt.
///
/// Each node retains its original jitter seed and the first retry is due
/// relative to this resumed poll. No network requests are performed.
pub fn plan_replica_retry_timeline_from(
    node_seeds: &[(&str, u64)],
    first_attempt: u32,
    attempts_per_node: u32,
    base_ms: u64,
    max_ms: u64,
    deadline: Duration,
) -> Result<Vec<ReplicaRetryEvent>, &'static str> {
    if node_seeds.is_empty()
        || node_seeds.len() > 64
        || first_attempt > 33
        || attempts_per_node > 33 - first_attempt
    {
        return Err("replica retry timeline exceeds limits");
    }
    // Validate even when the deadline or requested attempt count is zero.
    retry_delay(0, base_ms, max_ms, 0)?;
    for (index, &(node, _)) in node_seeds.iter().enumerate() {
        if node.is_empty()
            || node.len() > 64
            || !node
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || node_seeds[..index]
                .iter()
                .any(|&(previous, _)| previous == node)
        {
            return Err("invalid or duplicate replica retry node");
        }
    }

    // Every node's cumulative retry delays increase strictly. Merge these
    // already sorted streams with at most one queued event per node instead
    // of sorting the whole 64 x 33 event timeline.
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    let mut next_events = BinaryHeap::with_capacity(node_seeds.len());
    if attempts_per_node != 0 {
        for (index, &(node, seed)) in node_seeds.iter().enumerate() {
            let due_after = retry_delay(first_attempt, base_ms, max_ms, seed)?;
            if due_after <= deadline {
                next_events.push(Reverse((due_after, node, first_attempt, index)));
            }
        }
    }

    let mut timeline = Vec::with_capacity(node_seeds.len() * attempts_per_node as usize);
    while let Some(Reverse((due_after, node, attempt, index))) = next_events.pop() {
        timeline.push(ReplicaRetryEvent {
            node: node.to_owned(),
            attempt,
            due_after,
        });
        let next_attempt = attempt + 1;
        if next_attempt == first_attempt + attempts_per_node {
            continue;
        }
        let delay = retry_delay(next_attempt, base_ms, max_ms, node_seeds[index].1)?;
        if let Some(next_due) = due_after.checked_add(delay) {
            if next_due <= deadline {
                next_events.push(Reverse((next_due, node, next_attempt, index)));
            }
        }
    }
    Ok(timeline)
}

#[cfg(test)]
mod replica_retry_timeline_tests {
    use super::*;

    #[test]
    fn each_node_follows_exact_seeded_backoff_and_global_deadline() {
        let nodes = [("west", 3), ("east", 99), ("south", u64::MAX)];
        let deadline = Duration::from_secs(9);
        let timeline = plan_replica_retry_timeline(&nodes, 33, 100, 2000, deadline).unwrap();
        assert!(!timeline.is_empty());
        assert!(timeline.iter().all(|event| event.due_after <= deadline));
        assert!(timeline.windows(2).all(|pair| {
            (pair[0].due_after, pair[0].node.as_str(), pair[0].attempt)
                <= (pair[1].due_after, pair[1].node.as_str(), pair[1].attempt)
        }));
        for &(node, seed) in &nodes {
            let actual = timeline
                .iter()
                .filter(|event| event.node == node)
                .collect::<Vec<_>>();
            let expected = plan_retry_delays(33, 100, 2000, seed, deadline).unwrap();
            let mut elapsed = Duration::ZERO;
            assert_eq!(actual.len(), expected.len());
            for (index, (event, delay)) in actual.iter().zip(expected).enumerate() {
                elapsed += delay;
                assert_eq!(event.attempt, index as u32);
                assert_eq!(event.due_after, elapsed);
            }
        }
        let mut reversed = nodes;
        reversed.reverse();
        assert_eq!(
            timeline,
            plan_replica_retry_timeline(&reversed, 33, 100, 2000, deadline).unwrap(),
        );
    }

    #[test]
    fn boundary_deadlines_and_invalid_configuration_are_strict() {
        let nodes = [("a", 0)];
        let first = retry_delay(0, 100, 1000, 0).unwrap();
        assert!(
            plan_replica_retry_timeline(&nodes, 4, 100, 1000, Duration::ZERO)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            plan_replica_retry_timeline(&nodes, 4, 100, 1000, first)
                .unwrap()
                .len(),
            1
        );
        assert!(plan_replica_retry_timeline(&nodes, 34, 100, 1000, first).is_err());
        assert!(plan_replica_retry_timeline(&nodes, 0, 0, 1000, first).is_err());
        assert!(plan_replica_retry_timeline(&[("a", 0), ("a", 1)], 0, 100, 1000, first).is_err());
        assert!(plan_replica_retry_timeline(&[("bad/name", 0)], 0, 100, 1000, first).is_err());
    }
}

#[cfg(test)]
mod heap_retry_timeline_equivalence_tests {
    use super::*;

    #[test]
    fn heap_merge_matches_global_sort_for_full_and_partial_schedules() {
        let owned = (0..64u64)
            .map(|index| (format!("node_{index:02}"), index.wrapping_mul(1_000_003)))
            .collect::<Vec<_>>();
        let mut nodes = owned
            .iter()
            .map(|(name, seed)| (name.as_str(), *seed))
            .collect::<Vec<_>>();

        for deadline in [
            Duration::ZERO,
            Duration::from_millis(1),
            Duration::from_millis(300),
            Duration::from_secs(15),
            Duration::MAX,
        ] {
            for attempts in [0, 1, 12, 33] {
                let actual =
                    plan_replica_retry_timeline(&nodes, attempts, 100, 1_000, deadline).unwrap();
                let mut expected = Vec::new();
                for &(node, seed) in &nodes {
                    let mut due_after = Duration::ZERO;
                    for (attempt, delay) in plan_retry_delays(attempts, 100, 1_000, seed, deadline)
                        .unwrap()
                        .into_iter()
                        .enumerate()
                    {
                        due_after += delay;
                        expected.push(ReplicaRetryEvent {
                            node: node.to_owned(),
                            attempt: attempt as u32,
                            due_after,
                        });
                    }
                }
                expected.sort_unstable_by(|a, b| {
                    a.due_after
                        .cmp(&b.due_after)
                        .then_with(|| a.node.cmp(&b.node))
                        .then_with(|| a.attempt.cmp(&b.attempt))
                });
                assert_eq!(
                    actual, expected,
                    "attempts={attempts}, deadline={deadline:?}"
                );
                nodes.reverse();
                assert_eq!(
                    plan_replica_retry_timeline(&nodes, attempts, 100, 1_000, deadline).unwrap(),
                    expected
                );
                nodes.reverse();
            }
        }
    }
}

#[cfg(test)]
mod resumed_replica_retry_tests {
    use super::*;

    #[test]
    fn resumed_schedule_matches_the_exact_absolute_attempt_jitter() {
        for seed in [0, 7, 97, u64::MAX] {
            let all = plan_retry_delays(33, 100, 8_000, seed, Duration::MAX).unwrap();
            for first in [0, 1, 5, 16, 32, 33] {
                let resumed =
                    plan_retry_delays_from(first, 33 - first, 100, 8_000, seed, Duration::MAX)
                        .unwrap();
                assert_eq!(resumed, all[first as usize..]);
                let budget = resumed.iter().take(3).copied().sum::<Duration>();
                let partial =
                    plan_retry_delays_from(first, 33 - first, 100, 8_000, seed, budget).unwrap();
                assert_eq!(partial, resumed[..resumed.len().min(3)]);
            }
        }
    }

    #[test]
    fn rejects_invalid_attempt_windows_even_for_empty_deadlines() {
        for (start, count) in [(33, 1), (32, 2), (34, 0), (u32::MAX, 0)] {
            assert!(plan_retry_delays_from(start, count, 1, 2, 0, Duration::ZERO).is_err());
        }
        assert!(plan_retry_delays_from(33, 0, 0, 1, 0, Duration::ZERO).is_err());
        assert!(
            plan_retry_delays_from(0, 0, 1, 1, 0, Duration::ZERO)
                .unwrap()
                .is_empty()
        );
        assert!(
            plan_retry_delays_from(32, 1, 1, 1, 0, Duration::ZERO)
                .unwrap()
                .is_empty()
        );
    }
}

#[cfg(test)]
mod resumed_multi_replica_timeline_tests {
    use super::*;

    #[test]
    fn resumed_timeline_matches_absolute_single_node_backoff() {
        let peers = [("a", 7u64), ("b", 100u64), ("c", u64::MAX)];
        for first in [0, 1, 5, 16, 31, 32] {
            let attempts = 33 - first;
            let timeline = plan_replica_retry_timeline_from(
                &peers, first, attempts, 10, 500, Duration::from_secs(60),
            )
            .unwrap();
            for &(node, seed) in &peers {
                let per_node = timeline
                    .iter()
                    .filter(|event| event.node == node)
                    .collect::<Vec<_>>();
                let mut elapsed = Duration::ZERO;
                for (offset, event) in per_node.iter().enumerate() {
                    let attempt = first + offset as u32;
                    elapsed += retry_delay(attempt, 10, 500, seed).unwrap();
                    assert_eq!(event.attempt, attempt);
                    assert_eq!(event.due_after, elapsed);
                }
                assert_eq!(per_node.len(), attempts as usize);
            }
            assert!(timeline.windows(2).all(|pair| {
                (pair[0].due_after, pair[0].node.as_str(), pair[0].attempt)
                    <= (pair[1].due_after, pair[1].node.as_str(), pair[1].attempt)
            }));
        }
    }

    #[test]
    fn default_schedule_and_resumed_zero_epoch_are_identical() {
        let peers = [("west", 1), ("east", 42)];
        let deadline = Duration::from_secs(4);
        assert_eq!(
            plan_replica_retry_timeline(&peers, 33, 25, 400, deadline),
            plan_replica_retry_timeline_from(&peers, 0, 33, 25, 400, deadline)
        );
        assert!(plan_replica_retry_timeline_from(
            &peers, 32, 2, 1, 5, Duration::from_secs(1)
        ).is_err());
        assert!(plan_replica_retry_timeline_from(
            &peers, 34, 0, 1, 5, Duration::ZERO
        ).is_err());
        assert!(plan_replica_retry_timeline_from(
            &peers, 33, 0, 1, 5, Duration::ZERO
        ).unwrap().is_empty());
    }
}
