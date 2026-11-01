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
            || replicas > 16
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
        let mut acknowledgement_mask = 0u16;
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
                acknowledgement_mask |= 1u16 << index;
            }
        } else {
            // At most 16 members can be configured. A stack-backed sorted
            // index avoids allocating a HashMap for repeated acknowledgement
            // polls and still preserves the original membership bit positions.
            let mut membership = [("", 0usize); 16];
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
                acknowledgement_mask |= 1u16 << sorted[index].1;
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
mod tests {
    use super::*;

    #[test]
    fn accepts_intersecting_quorums() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        assert!(policy.can_write(2));
        assert!(!policy.can_read(1));
        assert!(!policy.can_write(4));
        assert_eq!(QuorumPolicy::new(5, 2, 4).unwrap().writes, 4);
    }

    #[test]
    fn counts_distinct_known_acknowledgements_only() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let members = ["a", "b", "c"];
        let single = policy.observe(&members, &["a", "a", "a"]).unwrap();
        assert_eq!(single.distinct_acks, 1);
        assert!(!single.can_read);
        assert!(!single.can_write);
        let majority = policy.observe(&members, &["a", "b", "b"]).unwrap();
        assert_eq!(majority.distinct_acks, 2);
        assert!(majority.can_read);
        assert!(majority.can_write);
        assert!(policy.observe(&members, &["unknown"]).is_err());
        assert!(policy.observe(&["a", "a", "c"], &["a"]).is_err());
        assert!(policy.observe(&["a", "b"], &["a"]).is_err());
        assert!(policy.observe(&["a", "b", ""], &["a"]).is_err());
        assert!(policy.observe(&members, &vec!["a"; 257]).is_err());
    }

    #[test]
    fn small_and_large_memberships_count_votes_without_duplicates() {
        for replicas in [1usize, 2, 3, 5, 8, 9, 16] {
            let names = (0..replicas)
                .map(|index| format!("node_{index}"))
                .collect::<Vec<_>>();
            let members = names.iter().map(String::as_str).collect::<Vec<_>>();
            let policy = QuorumPolicy::new(replicas, 1, replicas).unwrap();
            let all = policy.observe(&members, &members).unwrap();
            assert_eq!(all.distinct_acks, replicas);
            assert!(all.can_read && all.can_write);
            let repeated = vec![members[0]; 256];
            assert_eq!(
                policy.observe(&members, &repeated).unwrap().distinct_acks,
                1
            );
            assert!(policy.observe(&members, &["missing"]).is_err());
            let mut duplicates = members.clone();
            if replicas > 1 {
                duplicates[replicas - 1] = duplicates[0];
                assert!(policy.observe(&duplicates, &[]).is_err());
            }
        }
    }

    #[test]
    fn refuses_non_intersecting_or_unbounded_policies() {
        assert!(QuorumPolicy::new(3, 1, 2).is_err());
        assert!(QuorumPolicy::new(4, 2, 2).is_err());
        assert!(QuorumPolicy::new(0, 0, 0).is_err());
        assert!(QuorumPolicy::new(17, 9, 9).is_err());
    }
    #[test]
    fn all_accepted_quorum_policies_intersect() {
        for replicas in 1..=16 {
            for reads in 1..=replicas {
                for writes in 1..=replicas {
                    let policy = QuorumPolicy::new(replicas, reads, writes);
                    let safe = reads + writes > replicas && writes > replicas / 2;
                    assert_eq!(policy.is_ok(), safe, "{replicas}:{reads}:{writes}");
                    if let Ok(policy) = policy {
                        assert!(!policy.can_read(reads - 1));
                        assert!(policy.can_read(reads));
                        assert!(!policy.can_write(writes - 1));
                        assert!(policy.can_write(writes));
                        assert!(!policy.can_write(replicas + 1));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod quorum_deficit_tests {
    use super::*;

    #[test]
    fn counts_missing_votes_without_double_counting_duplicates() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let members = ["a", "b", "c"];
        assert_eq!(
            policy.deficits(&members, &["a", "a"]).unwrap(),
            QuorumDeficit {
                distinct_acks: 1,
                missing_reads: 1,
                missing_writes: 1
            }
        );
        assert_eq!(
            policy
                .deficits(&members, &["a", "b"])
                .unwrap()
                .missing_writes,
            0
        );
        assert!(policy.deficits(&members, &["unknown"]).is_err());
    }
}

#[cfg(test)]
mod direct_policy_validation_tests {
    use super::*;

    #[test]
    fn publicly_constructed_invalid_policy_cannot_report_quorum() {
        let unsafe_policy = QuorumPolicy {
            replicas: 3,
            reads: 0,
            writes: 1,
        };
        assert!(unsafe_policy.observe(&["a", "b", "c"], &["a"]).is_err());
        assert!(!unsafe_policy.can_read(3));
        assert!(!unsafe_policy.can_write(3));
    }
}

#[cfg(test)]
mod observed_quorum_fast_path_tests {
    use super::*;

    #[test]
    fn observations_match_checked_policy_for_all_quorum_thresholds() {
        for replicas in 1..=16 {
            let names = (0..replicas)
                .map(|index| format!("replica_{index}"))
                .collect::<Vec<_>>();
            let members = names.iter().map(String::as_str).collect::<Vec<_>>();
            for writes in 1..=replicas {
                for reads in 1..=replicas {
                    let Ok(policy) = QuorumPolicy::new(replicas, reads, writes) else {
                        continue;
                    };
                    for ack_count in 0..=replicas {
                        let observed = policy.observe(&members, &members[..ack_count]).unwrap();
                        assert_eq!(observed.distinct_acks, ack_count);
                        assert_eq!(observed.can_read, policy.can_read(ack_count));
                        assert_eq!(observed.can_write, policy.can_write(ack_count));
                    }
                }
            }
        }
    }
}

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
    let mut previous_mask = 0u16;
    let mut next_mask = 0u16;
    if acknowledgements.len() > 32 {
        // Large retry batches often repeat the same acknowledgement. Resolve
        // a replica ID to its old/new bit positions only once, while sharing
        // nodes count as one vote in each membership configuration.
        // A policy has at most sixteen members on each side. Build the
        // combined old/new ID index on the stack, then binary-search up to
        // 256 acknowledgements without allocating a hash table.
        let mut entries = [("", 0u16, 0u16); 32];
        let mut len = 0usize;
        for (index, &member) in previous_members.iter().enumerate() {
            if !valid_member(member) {
                return Err("invalid or duplicate quorum member");
            }
            entries[len] = (member, 1u16 << index, 0);
            len += 1;
        }
        for (index, &member) in next_members.iter().enumerate() {
            if !valid_member(member) {
                return Err("invalid or duplicate quorum member");
            }
            entries[len] = (member, 0, 1u16 << index);
            len += 1;
        }
        entries[..len].sort_unstable_by(|left, right| left.0.cmp(right.0));
        let mut index = [("", 0u16, 0u16); 32];
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
                previous_mask |= 1u16 << index;
            }
            if let Some(index) = new {
                next_mask |= 1u16 << index;
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
mod joint_reconfiguration_tests {
    use super::*;

    #[test]
    fn requires_both_old_and_new_acks_throughout_membership_change() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let old = ["a", "b", "c"];
        let new = ["b", "c", "d"];
        let old_only = observe_joint_quorum(policy, &old, policy, &new, &["a", "b"]).unwrap();
        assert_eq!(old_only.previous.distinct_acks, 2);
        assert_eq!(old_only.next.distinct_acks, 1);
        assert!(!old_only.can_write);
        let new_only = observe_joint_quorum(policy, &old, policy, &new, &["c", "d"]).unwrap();
        assert_eq!(new_only.previous.distinct_acks, 1);
        assert_eq!(new_only.next.distinct_acks, 2);
        assert!(!new_only.can_write);
        let both = observe_joint_quorum(policy, &old, policy, &new, &["a", "b", "d", "b"]).unwrap();
        assert!(both.can_read && both.can_write);
        assert_eq!(both.previous.distinct_acks, 2);
        assert_eq!(both.next.distinct_acks, 2);
        assert!(observe_joint_quorum(policy, &old, policy, &new, &["unknown"]).is_err());
    }

    #[test]
    fn validates_both_memberships_and_quorum_policies() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        assert!(
            observe_joint_quorum(policy, &["a", "a", "c"], policy, &["b", "c", "d"], &[]).is_err()
        );
        assert!(
            observe_joint_quorum(policy, &["a", "b", "c"], policy, &["b", "b", "d"], &[]).is_err()
        );
        let unsafe_policy = QuorumPolicy {
            replicas: 3,
            reads: 1,
            writes: 1,
        };
        assert!(
            observe_joint_quorum(
                unsafe_policy,
                &["a", "b", "c"],
                policy,
                &["b", "c", "d"],
                &[]
            )
            .is_err()
        );
        let votes = vec!["b"; 257];
        assert!(
            observe_joint_quorum(policy, &["a", "b", "c"], policy, &["b", "c", "d"], &votes)
                .is_err()
        );
    }

    #[test]
    fn counts_shared_members_once_in_each_large_configuration() {
        let old_ids = (0..16).map(|i| format!("n{i}")).collect::<Vec<_>>();
        let new_ids = (8..24).map(|i| format!("n{i}")).collect::<Vec<_>>();
        let old = old_ids.iter().map(String::as_str).collect::<Vec<_>>();
        let new = new_ids.iter().map(String::as_str).collect::<Vec<_>>();
        let policy = QuorumPolicy::new(16, 9, 9).unwrap();
        let acknowledgements = (0..16)
            .map(|i| format!("n{i}"))
            .chain((16..24).map(|i| format!("n{i}")))
            .collect::<Vec<_>>();
        let votes = acknowledgements
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let result = observe_joint_quorum(policy, &old, policy, &new, &votes).unwrap();
        assert_eq!(result.previous.distinct_acks, 16);
        assert_eq!(result.next.distinct_acks, 16);
        assert!(result.can_write);
    }
}

#[cfg(test)]
mod joint_quorum_large_ack_index_tests {
    use super::*;

    #[test]
    fn indexed_acknowledgements_match_small_batch_observations() {
        let policy = QuorumPolicy::new(16, 9, 9).unwrap();
        let old_nodes = (0..16)
            .map(|index| format!("node_{index:02}"))
            .collect::<Vec<_>>();
        let new_nodes = (8..24)
            .map(|index| format!("node_{index:02}"))
            .collect::<Vec<_>>();
        let old = old_nodes.iter().map(String::as_str).collect::<Vec<_>>();
        let new = new_nodes.iter().map(String::as_str).collect::<Vec<_>>();
        for count in 0..=24usize {
            let acknowledgements = (0..count)
                .map(|index| format!("node_{index:02}"))
                .collect::<Vec<_>>();
            let unique = acknowledgements
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            let mut repeated = Vec::new();
            for _ in 0..10 {
                repeated.extend(unique.iter().copied());
            }
            if repeated.len() <= 32 {
                continue;
            }
            let expected = observe_joint_quorum(policy, &old, policy, &new, &unique).unwrap();
            assert_eq!(
                observe_joint_quorum(policy, &old, policy, &new, &repeated).unwrap(),
                expected,
                "distinct acknowledgements: {count}",
            );
        }
        let too_many = vec!["node_08"; 257];
        assert!(observe_joint_quorum(policy, &old, policy, &new, &too_many).is_err());
        let mut bad = vec!["node_08"; 40];
        bad.push("stranger");
        assert!(observe_joint_quorum(policy, &old, policy, &new, &bad).is_err());
    }
}

#[cfg(test)]
mod indexed_joint_membership_validation_tests {
    use super::*;

    #[test]
    fn large_ack_batches_validate_both_memberships_and_count_unique_votes() {
        let policy = QuorumPolicy::new(9, 5, 5).unwrap();
        let previous = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
        let next = ["b", "c", "d", "e", "f", "g", "h", "i", "j"];
        let acks = (0..128)
            .map(|index| if index % 2 == 0 { "a" } else { "j" })
            .collect::<Vec<_>>();
        let observed = observe_joint_quorum(policy, &previous, policy, &next, &acks).unwrap();
        assert_eq!(observed.previous.distinct_acks, 1);
        assert_eq!(observed.next.distinct_acks, 1);
        assert!(!observed.can_write);
        let mut invalid_old = previous;
        invalid_old[8] = "a";
        assert!(observe_joint_quorum(policy, &invalid_old, policy, &next, &acks).is_err());
        let mut invalid_new = next;
        invalid_new[8] = "b";
        assert!(observe_joint_quorum(policy, &previous, policy, &invalid_new, &acks).is_err());
        let mut bad_ack = acks;
        bad_ack[127] = "unknown";
        assert!(observe_joint_quorum(policy, &previous, policy, &next, &bad_ack).is_err());
    }
}

#[cfg(test)]
mod sparse_large_quorum_ack_tests {
    use super::*;

    #[test]
    fn sparse_acks_match_full_member_count_and_preserve_errors() {
        let nodes = (0..16)
            .map(|index| format!("replica_{index}"))
            .collect::<Vec<_>>();
        let names = nodes.iter().map(String::as_str).collect::<Vec<_>>();
        let policy = QuorumPolicy::new(16, 8, 9).unwrap();
        for count in [0, 1, 2, 7, 8, 9, 16] {
            let observed = policy.observe(&names, &names[..count]).unwrap();
            assert_eq!(observed.distinct_acks, count);
            assert_eq!(observed.can_read, count >= 8);
            assert_eq!(observed.can_write, count >= 9);
        }
        assert_eq!(
            policy
                .observe(&names, &["replica_0"; 8])
                .unwrap()
                .distinct_acks,
            1
        );
        assert!(policy.observe(&names, &["missing"]).is_err());
        let mut duplicate = names.clone();
        duplicate[15] = duplicate[0];
        assert!(policy.observe(&duplicate, &[]).is_err());
        assert!(policy.observe(&duplicate, &names[..9]).is_err());
    }
}

#[cfg(test)]
mod stack_quorum_index_tests {
    use super::*;

    #[test]
    fn many_acknowledgements_match_reference_masks_in_every_large_group() {
        for replicas in 9..=16 {
            let names = (0..replicas)
                .map(|n| format!("member_{n}"))
                .collect::<Vec<_>>();
            let members = names.iter().map(String::as_str).collect::<Vec<_>>();
            let policy = QuorumPolicy::new(replicas, 1, replicas).unwrap();
            for seed in 0..32usize {
                let acks = (0..256)
                    .map(|n| members[(n * 37 + seed) % replicas])
                    .collect::<Vec<_>>();
                let reference = (0..replicas)
                    .filter(|&i| acks.contains(&members[i]))
                    .count();
                let observed = policy.observe(&members, &acks).unwrap();
                assert_eq!(observed.distinct_acks, reference);
                assert_eq!(observed.can_write, reference >= replicas);
            }
            let mut duplicates = members.clone();
            duplicates[replicas - 1] = duplicates[0];
            assert!(policy.observe(&duplicates, &members).is_err());
        }
    }
}

#[cfg(test)]
mod stack_joint_quorum_index_tests {
    use super::*;

    #[test]
    fn many_repeated_votes_match_independent_old_and_new_observations() {
        let previous = QuorumPolicy::new(16, 8, 9).unwrap();
        let next = QuorumPolicy::new(16, 8, 9).unwrap();
        let old = (0..16).map(|i| format!("old{i}")).collect::<Vec<_>>();
        let new = (8..24).map(|i| format!("old{i}")).collect::<Vec<_>>();
        let old_names = old.iter().map(String::as_str).collect::<Vec<_>>();
        let new_names = new.iter().map(String::as_str).collect::<Vec<_>>();
        let votes = (0..240)
            .map(|i| {
                if i % 2 == 0 {
                    old_names[(i / 2) % 16]
                } else {
                    new_names[(i / 2) % 16]
                }
            })
            .collect::<Vec<_>>();
        let observed =
            observe_joint_quorum(previous, &old_names, next, &new_names, &votes).unwrap();
        let old_votes = votes
            .iter()
            .copied()
            .filter(|v| old_names.contains(v))
            .collect::<Vec<_>>();
        let new_votes = votes
            .iter()
            .copied()
            .filter(|v| new_names.contains(v))
            .collect::<Vec<_>>();
        assert_eq!(
            observed.previous,
            previous.observe(&old_names, &old_votes).unwrap()
        );
        assert_eq!(observed.next, next.observe(&new_names, &new_votes).unwrap());
        assert!(observed.can_write);
        assert!(observed.can_read);
    }

    #[test]
    fn indexed_validation_rejects_duplicate_members_and_unknown_votes() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let votes = vec!["a"; 33];
        assert!(
            observe_joint_quorum(policy, &["a", "a", "b"], policy, &["a", "b", "c"], &votes,)
                .is_err()
        );
        assert!(
            observe_joint_quorum(
                policy,
                &["a", "b", "c"],
                policy,
                &["a", "b", "c"],
                &vec!["unknown"; 33],
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod failure_zone_quorum_reference_tests {
    use super::*;
    use crate::distributed_membership::Member;

    fn membership(zones: &[usize]) -> Vec<Member> {
        zones
            .iter()
            .enumerate()
            .flat_map(|(zone, &count)| {
                (0..count).map(move |index| Member {
                    node: format!("node_{zone}_{index}"),
                    zone: format!("zone_{zone}"),
                    weight: 1,
                })
            })
            .collect()
    }

    #[test]
    fn worst_case_matches_exhaustive_zone_failure_subsets() {
        for counts in [
            &[1usize, 1, 1][..],
            &[1, 2, 1, 1],
            &[4, 1, 2],
            &[2, 3, 1, 2, 1],
            &[3, 3, 3, 2, 3, 2],
        ] {
            let members = membership(counts);
            let n = members.len();
            let writes = n / 2 + 1;
            let reads = n - writes + 1;
            let policy = QuorumPolicy::new(n, reads, writes).unwrap();
            for failed in 0..=counts.len() {
                let mut most_lost = 0usize;
                for subset in 0..(1usize << counts.len()) {
                    if subset.count_ones() as usize != failed {
                        continue;
                    }
                    let lost = counts
                        .iter()
                        .enumerate()
                        .filter(|(zone, _)| subset & (1 << zone) != 0)
                        .map(|(_, &size)| size)
                        .sum::<usize>();
                    most_lost = most_lost.max(lost);
                }
                let expected = n - most_lost;
                let result = worst_case_zone_quorum(policy, &members, failed).unwrap();
                assert_eq!(
                    result.surviving_nodes, expected,
                    "zones={counts:?} failed={failed}"
                );
                assert_eq!(result.can_read, expected >= reads);
                assert_eq!(result.can_write, expected >= writes);
            }
        }
    }

    #[test]
    fn rejects_bad_membership_and_out_of_range_zone_failures() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let members = membership(&[2, 1]);
        assert!(worst_case_zone_quorum(policy, &members, 3).is_err());
        assert_eq!(
            worst_case_zone_quorum(policy, &members, 0)
                .unwrap()
                .surviving_nodes,
            3
        );
        let wrong = membership(&[2, 2]);
        assert!(worst_case_zone_quorum(policy, &wrong, 1).is_err());
        let mut repeated = members.clone();
        repeated[2].node = repeated[0].node.clone();
        assert!(worst_case_zone_quorum(policy, &repeated, 0).is_err());
    }
}

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
        if let Some(&shared) = candidates.iter().find(|name| {
            previous_members.contains(name) && next_members.contains(name)
        }) {
            return Ok(vec![shared.to_owned()]);
        }
    }
    // Policies permit at most 16 members on either side. A 32-bit mask
    // represents the complete candidate set without heap-backed DP paths.
    debug_assert!(candidates.len() <= 32);
    let width = next_missing + 1;
    let mut best = vec![None::<u32>; (previous_missing + 1) * width];
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
                let updated_mask = mask | (1u32 << index);
                let slot = &mut best[updated_old * width + updated_new];
                let replace = match *slot {
                    None => true,
                    Some(existing) if updated_mask.count_ones() < existing.count_ones() => true,
                    Some(existing) if updated_mask.count_ones() == existing.count_ones() => {
                        // Candidates are sorted. The set containing the first
                        // differing candidate is the lexicographically first.
                        let difference = updated_mask ^ existing;
                        difference != 0 && updated_mask & (1u32 << difference.trailing_zeros()) != 0
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
        .filter(|&(index, _)| selected & (1u32 << index) != 0)
        .map(|(_, &candidate)| candidate.to_owned())
        .collect())
}

#[cfg(test)]
mod optimal_joint_write_ack_tests {
    use super::*;

    #[test]
    fn shared_replica_reaches_both_quorums_with_one_new_vote() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let old = ["a", "b", "c"];
        let new = ["b", "c", "d"];
        assert_eq!(
            plan_joint_write_acknowledgements(policy, &old, policy, &new, &["a", "d"]).unwrap(),
            vec!["b"],
        );
        assert_eq!(
            plan_joint_write_acknowledgements(policy, &old, policy, &new, &["b"]).unwrap(),
            vec!["c"],
        );
        assert!(
            plan_joint_write_acknowledgements(policy, &old, policy, &new, &["a", "b", "c", "d"],)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn already_satisfied_membership_requires_only_missing_votes_from_other_side() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let previous = ["a", "b", "c"];
        let next = ["b", "c", "d"];
        assert_eq!(
            plan_joint_write_acknowledgements(policy, &previous, policy, &next, &["a", "b"])
                .unwrap(),
            ["c"]
        );
        assert_eq!(
            plan_joint_write_acknowledgements(policy, &previous, policy, &next, &["b", "d"])
                .unwrap(),
            ["a"]
        );
        assert_eq!(
            plan_joint_write_acknowledgements(policy, &previous, policy, &next, &["a", "d"])
                .unwrap(),
            ["b"]
        );
    }

    #[test]
    fn exhaustive_small_configurations_match_brute_force_minimum_and_ties() {
        let names = ["a", "b", "c", "d"];
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        for old in [["a", "b", "c"], ["a", "b", "d"], ["b", "c", "d"]] {
            for new in [["a", "b", "c"], ["a", "c", "d"], ["b", "c", "d"]] {
                for acknowledged_mask in 0u32..16 {
                    let acknowledged = names
                        .iter()
                        .enumerate()
                        .filter_map(|(i, &name)| {
                            (acknowledged_mask & (1 << i) != 0).then_some(name)
                        })
                        .collect::<Vec<_>>();
                    if !acknowledged
                        .iter()
                        .all(|name| old.contains(name) || new.contains(name))
                    {
                        continue;
                    }
                    let selected = plan_joint_write_acknowledgements(
                        policy,
                        &old,
                        policy,
                        &new,
                        &acknowledged,
                    )
                    .unwrap();
                    let mut expected: Option<Vec<&str>> = None;
                    for subset in 0u32..16 {
                        if subset & acknowledged_mask != 0 {
                            continue;
                        }
                        let extra = names
                            .iter()
                            .enumerate()
                            .filter_map(|(i, &name)| (subset & (1 << i) != 0).then_some(name))
                            .collect::<Vec<_>>();
                        if !extra
                            .iter()
                            .all(|name| old.contains(name) || new.contains(name))
                        {
                            continue;
                        }
                        let mut votes = acknowledged.clone();
                        votes.extend(&extra);
                        if !observe_joint_quorum(policy, &old, policy, &new, &votes)
                            .unwrap()
                            .can_write
                        {
                            continue;
                        }
                        if expected.as_ref().is_none_or(|best| {
                            extra.len() < best.len() || (extra.len() == best.len() && extra < *best)
                        }) {
                            expected = Some(extra);
                        }
                    }
                    assert_eq!(
                        selected,
                        expected
                            .unwrap()
                            .iter()
                            .map(|name| (*name).to_owned())
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }

    #[test]
    fn invalid_configurations_and_unknown_acknowledgements_fail_closed() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        assert!(
            plan_joint_write_acknowledgements(
                policy,
                &["a", "a", "c"],
                policy,
                &["b", "c", "d"],
                &[],
            )
            .is_err()
        );
        assert!(
            plan_joint_write_acknowledgements(
                policy,
                &["a", "b", "c"],
                policy,
                &["b", "c", "d"],
                &["x"],
            )
            .is_err()
        );
    }
}
