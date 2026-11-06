//! Quorum policy validation for future replicated storage writes.
//! This module does not implement consensus or distributed transactions.

fn valid_member(member: &str) -> bool {
    !member.is_empty()
        && member.len() <= 64
        && member
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuorumObservation {
    pub distinct_acks: usize,
    pub can_read: bool,
    pub can_write: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuorumPolicy {
    pub replicas: usize,
    pub reads: usize,
    pub writes: usize,
}

impl QuorumPolicy {
    /// Require overlapping read/write and write/write quorums.
    pub fn new(replicas: usize, reads: usize, writes: usize) -> Result<Self, &'static str> {
        if replicas == 0
            || replicas > 64
            || reads == 0
            || writes == 0
            || reads > replicas
            || writes > replicas
            || reads + writes <= replicas
            || writes <= replicas / 2
        {
            return Err("unsafe replica quorum policy");
        }
        Ok(Self {
            replicas,
            reads,
            writes,
        })
    }

    fn validate(self) -> Result<(), &'static str> {
        Self::new(self.replicas, self.reads, self.writes).map(|_| ())
    }

    /// Determine availability from actual replica IDs instead of trusting a
    /// caller-supplied acknowledgement count. Duplicate replies never count
    /// as more than one vote. This is planning, not a consensus protocol.
    pub fn observe(
        self,
        members: &[&str],
        acknowledgements: &[&str],
    ) -> Result<QuorumObservation, &'static str> {
        self.validate()?;
        if members.len() != self.replicas || acknowledgements.len() > 256 {
            return Err("quorum membership or acknowledgement count mismatch");
        }
        let mut acknowledgement_mask = 0u64;
        if members.len() <= 8 || acknowledgements.len() <= 8 {
            // Small replication groups are the common case. A short borrowed
            // lookup avoids allocating and hashing a temporary member map.
            for (index, &member) in members.iter().enumerate() {
                if !valid_member(member) || members[..index].contains(&member) {
                    return Err("invalid or duplicate quorum member");
                }
            }
            for &replica in acknowledgements {
                let index = members
                    .iter()
                    .position(|&member| member == replica)
                    .ok_or("acknowledgement from unknown replica")?;
                acknowledgement_mask |= 1u64 << index;
            }
        } else {
            // At most 64 members can be configured. A stack-backed sorted
            // index avoids allocating a HashMap for repeated acknowledgement
            // polls and still preserves the original membership bit positions.
            let mut membership = [("", 0usize); 64];
            for (index, &member) in members.iter().enumerate() {
                if !valid_member(member) {
                    return Err("invalid or duplicate quorum member");
                }
                membership[index] = (member, index);
            }
            let sorted = &mut membership[..members.len()];
            sorted.sort_unstable_by(|left, right| left.0.cmp(right.0));
            if sorted.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err("invalid or duplicate quorum member");
            }
            for &replica in acknowledgements {
                let index = sorted
                    .binary_search_by(|(member, _)| member.cmp(&replica))
                    .map_err(|_| "acknowledgement from unknown replica")?;
                acknowledgement_mask |= 1u64 << sorted[index].1;
            }
        }
        let distinct_acks = acknowledgement_mask.count_ones() as usize;
        Ok(QuorumObservation {
            distinct_acks,
            // Self::new was checked before membership processing, so these
            // comparisons do not need to validate the policy twice more.
            can_read: distinct_acks >= self.reads,
            can_write: distinct_acks >= self.writes,
        })
    }

    fn valid_ack_count(self, unique_acks: usize) -> bool {
        self.validate().is_ok() && unique_acks <= self.replicas
    }

    pub fn can_read(self, unique_acks: usize) -> bool {
        self.valid_ack_count(unique_acks) && unique_acks >= self.reads
    }

    pub fn can_write(self, unique_acks: usize) -> bool {
        self.valid_ack_count(unique_acks) && unique_acks >= self.writes
    }
}

/// Conservative availability after losing the largest failure domains.
/// This is a capacity and quorum assessment, not a consensus guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneFailureQuorum {
    pub failed_zones: usize,
    pub surviving_nodes: usize,
    pub can_read: bool,
    pub can_write: bool,
}

/// Evaluate the worst placement-independent loss of any `failed_zones`
/// distinct failure domains. Grouping by zone avoids enumerating all failure
/// subsets, and the read/write thresholds still use the original membership.
pub fn worst_case_zone_quorum(
    policy: QuorumPolicy,
    members: &[crate::distributed_membership::Member],
    failed_zones: usize,
) -> Result<ZoneFailureQuorum, &'static str> {
    policy.validate()?;
    crate::distributed_membership::validate_membership(members)?;
    if members.len() != policy.replicas {
        return Err("quorum policy does not match failure-zone membership");
    }
    let mut zones: Vec<(&str, usize)> = Vec::new();
    for member in members {
        if let Some((_, count)) = zones
            .iter_mut()
            .find(|(zone, _)| *zone == member.zone.as_str())
        {
            *count += 1;
        } else {
            zones.push((&member.zone, 1));
        }
    }
    if failed_zones > zones.len() {
        return Err("failure-zone count exceeds configured zones");
    }
    zones.sort_unstable_by_key(|&(_, count)| std::cmp::Reverse(count));
    let lost = zones
        .iter()
        .take(failed_zones)
        .map(|&(_, count)| count)
        .sum::<usize>();
    let surviving = members.len() - lost;
    Ok(ZoneFailureQuorum {
        failed_zones,
        surviving_nodes: surviving,
        can_read: policy.can_read(surviving),
        can_write: policy.can_write(surviving),
    })
}

/// Additional replica acknowledgements needed to satisfy a quorum policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuorumDeficit {
    pub distinct_acks: usize,
    pub missing_reads: usize,
    pub missing_writes: usize,
}

impl QuorumPolicy {
    /// This only counts replies from listed members; it does not implement consensus.
    pub fn deficits(
        self,
        members: &[&str],
        acknowledgements: &[&str],
    ) -> Result<QuorumDeficit, &'static str> {
        let observed = self.observe(members, acknowledgements)?;
        Ok(QuorumDeficit {
            distinct_acks: observed.distinct_acks,
            missing_reads: self.reads.saturating_sub(observed.distinct_acks),
            missing_writes: self.writes.saturating_sub(observed.distinct_acks),
        })
    }
}

#[cfg(test)]
#[path = "distributed_quorum_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/quorum_deficit_tests.rs"]
mod quorum_deficit_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/direct_policy_validation_tests.rs"]
mod direct_policy_validation_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/observed_quorum_fast_path_tests.rs"]
mod observed_quorum_fast_path_tests;

/**
 * Report whether acknowledgements satisfy both quorum configurations during
 * a membership change. This is a pure planner, not an election or consensus
 * implementation. One shared node counts once in each configuration.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JointQuorumObservation {
    pub previous: QuorumObservation,
    pub next: QuorumObservation,
    pub can_read: bool,
    pub can_write: bool,
}

pub fn observe_joint_quorum(
    previous: QuorumPolicy,
    previous_members: &[&str],
    next: QuorumPolicy,
    next_members: &[&str],
    acknowledgements: &[&str],
) -> Result<JointQuorumObservation, &'static str> {
    // Large batches index both memberships together below. Validate their
    // policies here, but avoid building two temporary membership hash maps.
    previous.validate()?;
    next.validate()?;
    if previous_members.len() != previous.replicas || next_members.len() != next.replicas {
        return Err("quorum membership or acknowledgement count mismatch");
    }
    if acknowledgements.len() > 256 {
        return Err("quorum acknowledgement count exceeds bound");
    }
    if acknowledgements.len() <= 32 {
        // Small acknowledgement batches use borrowed linear probes. Validate
        // both memberships even when no acknowledgements were received.
        previous.observe(previous_members, &[])?;
        next.observe(next_members, &[])?;
    }
    let mut previous_mask = 0u64;
    let mut next_mask = 0u64;
    if acknowledgements.len() > 32 {
        // Large retry batches often repeat the same acknowledgement. Resolve
        // a replica ID to its old/new bit positions only once, while sharing
        // nodes count as one vote in each membership configuration.
        // A policy has at most 64 members on each side. Build the
        // combined old/new ID index on the stack, then binary-search up to
        // 256 acknowledgements without allocating a hash table.
        let mut entries = [("", 0u64, 0u64); 128];
        let mut len = 0usize;
        for (index, &member) in previous_members.iter().enumerate() {
            if !valid_member(member) {
                return Err("invalid or duplicate quorum member");
            }
            entries[len] = (member, 1u64 << index, 0);
            len += 1;
        }
        for (index, &member) in next_members.iter().enumerate() {
            if !valid_member(member) {
                return Err("invalid or duplicate quorum member");
            }
            entries[len] = (member, 0, 1u64 << index);
            len += 1;
        }
        entries[..len].sort_unstable_by(|left, right| left.0.cmp(right.0));
        let mut index = [("", 0u64, 0u64); 128];
        let mut unique = 0usize;
        for (member, old, new) in entries[..len].iter().copied() {
            if unique > 0 && index[unique - 1].0 == member {
                let previous = &mut index[unique - 1];
                // Duplicates across configurations are legal; duplicates
                // within the same configuration must still be rejected.
                if (old != 0 && previous.1 != 0) || (new != 0 && previous.2 != 0) {
                    return Err("invalid or duplicate quorum member");
                }
                previous.1 |= old;
                previous.2 |= new;
            } else {
                index[unique] = (member, old, new);
                unique += 1;
            }
        }
        let sorted = &index[..unique];
        for &member in acknowledgements {
            let position = sorted
                .binary_search_by(|entry| entry.0.cmp(member))
                .map_err(|_| "acknowledgement from unknown replica")?;
            previous_mask |= sorted[position].1;
            next_mask |= sorted[position].2;
        }
    } else {
        // Small batches avoid a temporary map entirely.
        for &member in acknowledgements {
            let old = previous_members.iter().position(|&id| id == member);
            let new = next_members.iter().position(|&id| id == member);
            if old.is_none() && new.is_none() {
                return Err("acknowledgement from unknown replica");
            }
            if let Some(index) = old {
                previous_mask |= 1u64 << index;
            }
            if let Some(index) = new {
                next_mask |= 1u64 << index;
            }
        }
    }
    let previous_count = previous_mask.count_ones() as usize;
    let next_count = next_mask.count_ones() as usize;
    let previous = QuorumObservation {
        distinct_acks: previous_count,
        can_read: previous_count >= previous.reads,
        can_write: previous_count >= previous.writes,
    };
    let next = QuorumObservation {
        distinct_acks: next_count,
        can_read: next_count >= next.reads,
        can_write: next_count >= next.writes,
    };
    Ok(JointQuorumObservation {
        previous,
        next,
        can_read: previous.can_read && next.can_read,
        can_write: previous.can_write && next.can_write,
    })
}

#[cfg(test)]
#[path = "distributed_quorum_tests/joint_reconfiguration_tests.rs"]
mod joint_reconfiguration_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/joint_quorum_large_ack_index_tests.rs"]
mod joint_quorum_large_ack_index_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/indexed_joint_membership_validation_tests.rs"]
mod indexed_joint_membership_validation_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/sparse_large_quorum_ack_tests.rs"]
mod sparse_large_quorum_ack_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/stack_quorum_index_tests.rs"]
mod stack_quorum_index_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/stack_joint_quorum_index_tests.rs"]
mod stack_joint_quorum_index_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/failure_zone_quorum_reference_tests.rs"]
mod failure_zone_quorum_reference_tests;

/// Pick the smallest set of additional replica acknowledgements that would
/// satisfy *both* write quorums during joint membership reconfiguration.
///
/// This is a planning helper, not a lease, acknowledgement, or consensus
/// operation. Ties choose the lexicographically earliest set of replica IDs.
/// A shared replica can satisfy one vote in each membership with one request.
pub fn plan_joint_write_acknowledgements(
    previous: QuorumPolicy,
    previous_members: &[&str],
    next: QuorumPolicy,
    next_members: &[&str],
    acknowledgements: &[&str],
) -> Result<Vec<String>, &'static str> {
    let observed = observe_joint_quorum(
        previous,
        previous_members,
        next,
        next_members,
        acknowledgements,
    )?;
    let previous_missing = previous
        .writes
        .saturating_sub(observed.previous.distinct_acks);
    let next_missing = next.writes.saturating_sub(observed.next.distinct_acks);
    if previous_missing == 0 && next_missing == 0 {
        return Ok(Vec::new());
    }

    let mut candidates = previous_members
        .iter()
        .chain(next_members)
        .copied()
        .filter(|member| !acknowledgements.contains(member))
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    // A membership that already has write quorum imposes no further
    // constraint. The lexicographically earliest remaining members from the
    // other membership are the exact minimum, without a two-dimensional DP.
    if previous_missing == 0 || next_missing == 0 {
        let (missing, members) = if previous_missing == 0 {
            (next_missing, next_members)
        } else {
            (previous_missing, previous_members)
        };
        return Ok(candidates
            .into_iter()
            .filter(|candidate| members.contains(candidate))
            .take(missing)
            .map(str::to_owned)
            .collect());
    }
    // One common unacknowledged member can supply the final vote in both
    // memberships. Prefer the smallest common name for the exact tie break.
    if previous_missing == 1 && next_missing == 1 {
        if let Some(&shared) = candidates
            .iter()
            .find(|&&name| previous_members.contains(&name) && next_members.contains(&name))
        {
            return Ok(vec![shared.to_owned()]);
        }
    }
    // Policies permit up to 64 members on either side. A 128-bit mask
    // represents every candidate without heap-backed DP paths.
    debug_assert!(candidates.len() <= 128);
    let width = next_missing + 1;
    let mut best = vec![None::<u128>; (previous_missing + 1) * width];
    best[0] = Some(0);
    for (index, candidate) in candidates.iter().enumerate() {
        let in_previous = usize::from(previous_members.contains(candidate));
        let in_next = usize::from(next_members.contains(candidate));
        for old in (0..=previous_missing).rev() {
            for new in (0..=next_missing).rev() {
                let Some(mask) = best[old * width + new] else {
                    continue;
                };
                let updated_old = (old + in_previous).min(previous_missing);
                let updated_new = (new + in_next).min(next_missing);
                if updated_old == old && updated_new == new {
                    continue;
                }
                let updated_mask = mask | (1u128 << index);
                let slot = &mut best[updated_old * width + updated_new];
                let replace = match *slot {
                    None => true,
                    Some(existing) if updated_mask.count_ones() < existing.count_ones() => true,
                    Some(existing) if updated_mask.count_ones() == existing.count_ones() => {
                        // Candidates are sorted. The set containing the first
                        // differing candidate is the lexicographically first.
                        let difference = updated_mask ^ existing;
                        difference != 0
                            && updated_mask & (1u128 << difference.trailing_zeros()) != 0
                    }
                    _ => false,
                };
                if replace {
                    *slot = Some(updated_mask);
                }
            }
        }
    }
    let selected = best[previous_missing * width + next_missing]
        .ok_or("joint write quorum cannot be reached")?;
    Ok(candidates
        .iter()
        .enumerate()
        .filter(|&(index, _)| selected & (1u128 << index) != 0)
        .map(|(_, &candidate)| candidate.to_owned())
        .collect())
}

#[cfg(test)]
#[path = "distributed_quorum_tests/optimal_joint_write_ack_tests.rs"]
mod optimal_joint_write_ack_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/wide_quorum_support_tests.rs"]
mod wide_quorum_support_tests;

/// Compute every worst-case failure-zone quorum outcome in a single pass.
///
/// The k-th entry describes losing the k largest distinct failure zones.
/// This is an availability envelope, not proof that any replica completed
/// consensus or that data written by a quorum was durably persisted.
pub fn zone_failure_quorum_curve(
    policy: QuorumPolicy,
    members: &[crate::distributed_membership::Member],
) -> Result<Vec<ZoneFailureQuorum>, &'static str> {
    policy.validate()?;
    crate::distributed_membership::validate_membership(members)?;
    if members.len() != policy.replicas {
        return Err("quorum policy does not match failure-zone membership");
    }
    // Validated quorum policies have at most 16 nodes, so accumulating
    // borrowed zone names is cheaper than building a hash table.
    let mut zones = Vec::<(&str, usize)>::new();
    for member in members {
        match zones
            .iter_mut()
            .find(|(zone, _)| *zone == member.zone.as_str())
        {
            Some((_, count)) => *count += 1,
            None => zones.push((&member.zone, 1)),
        }
    }
    zones.sort_unstable_by_key(|&(_, nodes)| std::cmp::Reverse(nodes));
    let mut surviving = members.len();
    let mut outcomes = Vec::with_capacity(zones.len() + 1);
    outcomes.push(ZoneFailureQuorum {
        failed_zones: 0,
        surviving_nodes: surviving,
        can_read: policy.can_read(surviving),
        can_write: policy.can_write(surviving),
    });
    for (index, (_, lost)) in zones.iter().enumerate() {
        surviving -= lost;
        outcomes.push(ZoneFailureQuorum {
            failed_zones: index + 1,
            surviving_nodes: surviving,
            can_read: policy.can_read(surviving),
            can_write: policy.can_write(surviving),
        });
    }
    Ok(outcomes)
}

/// Restore the fewest down failure domains needed to regain each
/// quorum threshold. Zone choices are sorted by survivor count, then name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRecoveryPlan {
    pub surviving_replicas: usize,
    pub restore_for_reads: Vec<String>,
    pub restore_for_writes: Vec<String>,
}

/// Plan availability recovery from a concrete set of failed zones.
///
/// Every listed zone must exist and be unique. This is only a quorum-capacity
/// calculation, not proof that recovering a node restores a durable snapshot.
pub fn plan_zone_quorum_recovery(
    policy: QuorumPolicy,
    members: &[crate::distributed_membership::Member],
    down_zones: &[&str],
) -> Result<ZoneRecoveryPlan, &'static str> {
    policy.validate()?;
    crate::distributed_membership::validate_membership(members)?;
    if members.len() != policy.replicas || down_zones.len() > members.len() {
        return Err("invalid zone recovery membership");
    }
    let mut zones = Vec::<(&str, usize)>::new();
    for member in members {
        if let Some((_, count)) = zones
            .iter_mut()
            .find(|(zone, _)| *zone == member.zone.as_str())
        {
            *count += 1;
        } else {
            zones.push((&member.zone, 1));
        }
    }
    let mut failed = Vec::<(&str, usize)>::new();
    for &zone in down_zones {
        let count = zones
            .iter()
            .find(|(known, _)| *known == zone)
            .ok_or("unknown failure zone")?
            .1;
        if failed.iter().any(|(known, _)| *known == zone) {
            return Err("duplicate failure zone");
        }
        failed.push((zone, count));
    }
    let lost = failed.iter().map(|(_, count)| *count).sum::<usize>();
    let surviving_replicas = members.len() - lost;
    failed.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    fn restore(failed: &[(&str, usize)], missing: usize) -> Vec<String> {
        let mut restored = Vec::new();
        let mut nodes = 0usize;
        for &(zone, count) in failed {
            if nodes >= missing {
                break;
            }
            restored.push(zone.to_owned());
            nodes += count;
        }
        restored
    }
    Ok(ZoneRecoveryPlan {
        surviving_replicas,
        restore_for_reads: restore(&failed, policy.reads.saturating_sub(surviving_replicas)),
        restore_for_writes: restore(&failed, policy.writes.saturating_sub(surviving_replicas)),
    })
}

#[cfg(test)]
#[path = "distributed_quorum_tests/actual_zone_quorum_recovery_tests.rs"]
mod actual_zone_quorum_recovery_tests;

#[cfg(test)]
#[path = "distributed_quorum_tests/batched_zone_failure_quorum_tests.rs"]
mod batched_zone_failure_quorum_tests;
