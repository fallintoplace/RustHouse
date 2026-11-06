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
#[path = "distributed_retry_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "distributed_retry_tests/retry_deadline_tests.rs"]
mod retry_deadline_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/total_retry_budget_tests.rs"]
mod total_retry_budget_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/retry_plan_single_validation_tests.rs"]
mod retry_plan_single_validation_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/zero_seed_retry_dispersion_tests.rs"]
mod zero_seed_retry_dispersion_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/planned_retry_consistency_tests.rs"]
mod planned_retry_consistency_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/subminimum_retry_budget_tests.rs"]
mod subminimum_retry_budget_tests;

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
    plan_replica_retry_timeline_from(node_seeds, 0, attempts_per_node, base_ms, max_ms, deadline)
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
    // A retry fan-out has at most 64 nodes. Validate all identifiers before
    // allocating an event heap. Larger fan-outs deduplicate borrowed names
    // using a stack sort instead of O(N^2) repeated string comparisons.
    let mut names = [""; 64];
    for (index, &(node, _)) in node_seeds.iter().enumerate() {
        if node.is_empty()
            || node.len() > 64
            || !node
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err("invalid or duplicate replica retry node");
        }
        if node_seeds.len() <= 8
            && node_seeds[..index]
                .iter()
                .any(|&(previous, _)| previous == node)
        {
            return Err("invalid or duplicate replica retry node");
        }
        names[index] = node;
    }
    if node_seeds.len() > 8 {
        let ordered = &mut names[..node_seeds.len()];
        ordered.sort_unstable();
        if ordered.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("invalid or duplicate replica retry node");
        }
    }
    // Empty or impossible retry timelines still reject invalid membership
    // and backoff limits above, without allocating any scheduling state.
    if attempts_per_node == 0 || deadline < Duration::from_millis(base_ms.div_ceil(2)) {
        return Ok(Vec::new());
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

    // Reserve only initially schedulable events, not the full retry horizon.
    let mut timeline = Vec::with_capacity(next_events.len());
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
#[path = "distributed_retry_tests/replica_retry_timeline_tests.rs"]
mod replica_retry_timeline_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/heap_retry_timeline_equivalence_tests.rs"]
mod heap_retry_timeline_equivalence_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/resumed_replica_retry_tests.rs"]
mod resumed_replica_retry_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/stack_retry_validation_tests.rs"]
mod stack_retry_validation_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/resumed_multi_replica_timeline_tests.rs"]
mod resumed_multi_replica_timeline_tests;

/// Persisted position in a deterministic, absolute-offset retry schedule.
///
/// `elapsed` is the exact sum of delays for attempts before `next_attempt`,
/// not wall-clock time since a process restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicaRetryCheckpoint<'a> {
    pub node: &'a str,
    pub seed: u64,
    pub next_attempt: u32,
    pub elapsed: Duration,
}

/// Resume the remaining per-node retry events from validated checkpoints.
///
/// A checkpoint with the wrong seed, attempt number or elapsed delay must not
/// silently replay a different schedule. The existing bounded heap planner
/// supplies the same deterministic global event order as an unbroken run.
pub fn plan_replica_retry_timeline_from_checkpoints(
    checkpoints: &[ReplicaRetryCheckpoint<'_>],
    attempts_per_node: u32,
    base_ms: u64,
    max_ms: u64,
    deadline: Duration,
) -> Result<Vec<ReplicaRetryEvent>, &'static str> {
    if checkpoints.is_empty() || checkpoints.len() > 64 || attempts_per_node > 33 {
        return Err("replica retry timeline exceeds limits");
    }
    for checkpoint in checkpoints {
        if checkpoint.next_attempt > attempts_per_node {
            return Err("replica retry checkpoint exceeds attempt budget");
        }
        let expected =
            (0..checkpoint.next_attempt).try_fold(Duration::ZERO, |elapsed, attempt| {
                let delay = retry_delay(attempt, base_ms, max_ms, checkpoint.seed)?;
                elapsed
                    .checked_add(delay)
                    .ok_or("replica retry checkpoint duration overflow")
            })?;
        if checkpoint.elapsed != expected {
            return Err("replica retry checkpoint does not match deterministic delays");
        }
    }
    let nodes = checkpoints
        .iter()
        .map(|checkpoint| (checkpoint.node, checkpoint.seed))
        .collect::<Vec<_>>();
    let mut events =
        plan_replica_retry_timeline(&nodes, attempts_per_node, base_ms, max_ms, deadline)?;
    events.retain(|event| {
        checkpoints.iter().any(|checkpoint| {
            event.node == checkpoint.node && event.attempt >= checkpoint.next_attempt
        })
    });
    Ok(events)
}

#[cfg(test)]
#[path = "distributed_retry_tests/resumed_timeline_checkpoint_tests.rs"]
mod resumed_timeline_checkpoint_tests;

#[cfg(test)]
#[path = "distributed_retry_tests/bounded_retry_timeline_allocation_tests.rs"]
mod bounded_retry_timeline_allocation_tests;

/// Admit planned retry events under shared and per-node sliding-window quotas.
///
/// Events are only selected, never executed. The window is half-open: an
/// event exactly one window after an older event may use its released slot.
/// Equal timestamps are resolved by attempt number and then node ID.
pub fn admit_retry_events_with_quotas(
    events: &[ReplicaRetryEvent],
    window: Duration,
    global_per_window: usize,
    node_per_window: usize,
) -> Result<Vec<ReplicaRetryEvent>, &'static str> {
    use std::collections::{HashMap, HashSet, VecDeque};

    if events.len() > 64 * 33 || window.is_zero() || global_per_window == 0 || node_per_window == 0
    {
        return Err("invalid retry admission limits");
    }
    let mut names = HashSet::new();
    let mut attempts = HashSet::new();
    for event in events {
        let node = event.node.as_str();
        if node.is_empty()
            || node.len() > 64
            || !node
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || event.attempt > 32
            || !attempts.insert((node, event.attempt))
        {
            return Err("invalid or duplicate retry event");
        }
        names.insert(node);
    }
    if names.len() > 64 {
        return Err("too many retry nodes");
    }
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_unstable_by(|left, right| {
        left.due_after
            .cmp(&right.due_after)
            .then_with(|| left.attempt.cmp(&right.attempt))
            .then_with(|| left.node.cmp(&right.node))
    });
    let mut global = VecDeque::<Duration>::new();
    let mut per_node = HashMap::<&str, VecDeque<Duration>>::new();
    let mut admitted = Vec::with_capacity(events.len());
    for event in ordered {
        while global
            .front()
            .is_some_and(|&start| event.due_after - start >= window)
        {
            global.pop_front();
        }
        let node_queue = per_node.entry(event.node.as_str()).or_default();
        while node_queue
            .front()
            .is_some_and(|&start| event.due_after - start >= window)
        {
            node_queue.pop_front();
        }
        if global.len() >= global_per_window || node_queue.len() >= node_per_window {
            continue;
        }
        global.push_back(event.due_after);
        node_queue.push_back(event.due_after);
        admitted.push(event.clone());
    }
    Ok(admitted)
}

#[cfg(test)]
#[path = "distributed_retry_tests/retry_admission_window_tests.rs"]
mod retry_admission_window_tests;

/// Local scheduling state for adaptive replica retry admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaCircuitPhase {
    Closed,
    Open,
    HalfOpen,
}

/// Deterministic circuit breaker. It does not run network operations or
/// coordinate across processes; persist updates with transfer outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicaCircuitBreaker {
    phase: ReplicaCircuitPhase,
    failure_threshold: u16,
    consecutive_failures: u16,
    reopen_count: u16,
    base_cooldown_ms: u64,
    max_cooldown_ms: u64,
    retry_at_ms: u64,
    last_ms: u64,
}

impl ReplicaCircuitBreaker {
    pub fn new(
        failure_threshold: u16,
        base_cooldown_ms: u64,
        max_cooldown_ms: u64,
        at_ms: u64,
    ) -> Result<Self, &'static str> {
        if failure_threshold == 0
            || failure_threshold > 1000
            || base_cooldown_ms == 0
            || max_cooldown_ms < base_cooldown_ms
        {
            return Err("invalid replica circuit breaker configuration");
        }
        Ok(Self {
            phase: ReplicaCircuitPhase::Closed,
            failure_threshold,
            consecutive_failures: 0,
            reopen_count: 0,
            base_cooldown_ms,
            max_cooldown_ms,
            retry_at_ms: 0,
            last_ms: at_ms,
        })
    }

    pub fn phase(&self) -> ReplicaCircuitPhase {
        self.phase
    }

    pub fn retry_at_ms(&self) -> Option<u64> {
        (self.phase == ReplicaCircuitPhase::Open).then_some(self.retry_at_ms)
    }

    /// Admit normal traffic when closed, or exactly one half-open probe.
    /// Repeated polls during the probe cannot admit another request.
    pub fn permit(&mut self, now_ms: u64) -> Result<bool, &'static str> {
        if now_ms < self.last_ms {
            return Err("replica circuit breaker clock moved backwards");
        }
        self.last_ms = now_ms;
        match self.phase {
            ReplicaCircuitPhase::Closed => Ok(true),
            ReplicaCircuitPhase::Open if now_ms >= self.retry_at_ms => {
                self.phase = ReplicaCircuitPhase::HalfOpen;
                Ok(true)
            }
            ReplicaCircuitPhase::Open | ReplicaCircuitPhase::HalfOpen => Ok(false),
        }
    }

    /// Record a successful request. The half-open probe closes the breaker.
    pub fn record_success(&mut self, now_ms: u64) -> Result<(), &'static str> {
        if now_ms < self.last_ms {
            return Err("replica circuit breaker clock moved backwards");
        }
        if self.phase == ReplicaCircuitPhase::Open {
            return Err("cannot complete a request while circuit is open");
        }
        self.last_ms = now_ms;
        self.phase = ReplicaCircuitPhase::Closed;
        self.consecutive_failures = 0;
        self.reopen_count = 0;
        Ok(())
    }

    /// Record a failure without mutating state if the next cooldown overflows.
    pub fn record_failure(&mut self, now_ms: u64) -> Result<(), &'static str> {
        if now_ms < self.last_ms {
            return Err("replica circuit breaker clock moved backwards");
        }
        if self.phase == ReplicaCircuitPhase::Open {
            return Err("cannot complete a request while circuit is open");
        }
        let failures = self.consecutive_failures.saturating_add(1);
        let trips = self.reopen_count;
        let open =
            self.phase == ReplicaCircuitPhase::HalfOpen || failures >= self.failure_threshold;
        let retry_at = if open {
            let shift = u32::from(trips.min(63));
            let cooldown = self
                .base_cooldown_ms
                .saturating_mul(1u64 << shift)
                .min(self.max_cooldown_ms);
            now_ms
                .checked_add(cooldown)
                .ok_or("replica circuit breaker cooldown overflow")?
        } else {
            self.retry_at_ms
        };
        self.last_ms = now_ms;
        self.consecutive_failures = failures;
        if open {
            self.phase = ReplicaCircuitPhase::Open;
            self.retry_at_ms = retry_at;
            self.reopen_count = trips.saturating_add(1);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "distributed_retry_tests/replica_circuit_tests.rs"]
mod replica_circuit_tests;
