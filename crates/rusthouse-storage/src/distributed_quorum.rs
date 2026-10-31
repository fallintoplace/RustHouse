//! Quorum policy validation for future replicated storage writes.
//! This module does not implement consensus or distributed transactions.

use std::collections::HashMap;

fn valid_member(member: &str) -> bool {
    !member.is_empty()
        && member.len() <= 64
        && member
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn member_index<'a>(members: &[&'a str]) -> Result<HashMap<&'a str, usize>, &'static str> {
    let mut membership = HashMap::with_capacity(members.len());
    for (index, &member) in members.iter().enumerate() {
        if !valid_member(member) || membership.insert(member, index).is_some() {
            return Err("invalid or duplicate quorum member");
        }
    }
    Ok(membership)
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
        if members.len() <= 8 {
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
            let membership = member_index(members)?;
            for &replica in acknowledgements {
                let Some(&index) = membership.get(replica) else {
                    return Err("acknowledgement from unknown replica");
                };
                acknowledgement_mask |= 1u16 << index;
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
        self.validate().is_ok()
            && unique_acks <= self.replicas
    }

    pub fn can_read(self, unique_acks: usize) -> bool {
        self.valid_ack_count(unique_acks) && unique_acks >= self.reads
    }

    pub fn can_write(self, unique_acks: usize) -> bool {
        self.valid_ack_count(unique_acks) && unique_acks >= self.writes
    }
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
            let names = (0..replicas).map(|index| format!("node_{index}")).collect::<Vec<_>>();
            let members = names.iter().map(String::as_str).collect::<Vec<_>>();
            let policy = QuorumPolicy::new(replicas, 1, replicas).unwrap();
            let all = policy.observe(&members, &members).unwrap();
            assert_eq!(all.distinct_acks, replicas);
            assert!(all.can_read && all.can_write);
            let repeated = vec![members[0]; 256];
            assert_eq!(policy.observe(&members, &repeated).unwrap().distinct_acks, 1);
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
            QuorumDeficit { distinct_acks: 1, missing_reads: 1, missing_writes: 1 }
        );
        assert_eq!(policy.deficits(&members, &["a", "b"]).unwrap().missing_writes, 0);
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
        let mut positions = HashMap::<&str, (u16, u16)>::with_capacity(
            previous_members.len() + next_members.len(),
        );
        for (index, &member) in previous_members.iter().enumerate() {
            if !valid_member(member) {
                return Err("invalid or duplicate quorum member");
            }
            let entry = positions.entry(member).or_default();
            if entry.0 != 0 {
                return Err("invalid or duplicate quorum member");
            }
            entry.0 = 1u16 << index;
        }
        for (index, &member) in next_members.iter().enumerate() {
            if !valid_member(member) {
                return Err("invalid or duplicate quorum member");
            }
            let entry = positions.entry(member).or_default();
            if entry.1 != 0 {
                return Err("invalid or duplicate quorum member");
            }
            entry.1 = 1u16 << index;
        }
        for &member in acknowledgements {
            let &(old, new) = positions.get(member)
                .ok_or("acknowledgement from unknown replica")?;
            previous_mask |= old;
            next_mask |= new;
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
        assert!(observe_joint_quorum(policy, &["a", "a", "c"],
            policy, &["b", "c", "d"], &[]).is_err());
        assert!(observe_joint_quorum(policy, &["a", "b", "c"],
            policy, &["b", "b", "d"], &[]).is_err());
        let unsafe_policy = QuorumPolicy { replicas: 3, reads: 1, writes: 1 };
        assert!(observe_joint_quorum(unsafe_policy, &["a", "b", "c"],
            policy, &["b", "c", "d"], &[]).is_err());
        let votes = vec!["b"; 257];
        assert!(observe_joint_quorum(policy, &["a", "b", "c"],
            policy, &["b", "c", "d"], &votes).is_err());
    }

    #[test]
    fn counts_shared_members_once_in_each_large_configuration() {
        let old_ids = (0..16).map(|i| format!("n{i}")).collect::<Vec<_>>();
        let new_ids = (8..24).map(|i| format!("n{i}")).collect::<Vec<_>>();
        let old = old_ids.iter().map(String::as_str).collect::<Vec<_>>();
        let new = new_ids.iter().map(String::as_str).collect::<Vec<_>>();
        let policy = QuorumPolicy::new(16, 9, 9).unwrap();
        let acknowledgements = (0..16).map(|i| format!("n{i}"))
            .chain((16..24).map(|i| format!("n{i}"))).collect::<Vec<_>>();
        let votes = acknowledgements.iter().map(String::as_str).collect::<Vec<_>>();
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
        let old_nodes = (0..16).map(|index| format!("node_{index:02}"))
            .collect::<Vec<_>>();
        let new_nodes = (8..24).map(|index| format!("node_{index:02}"))
            .collect::<Vec<_>>();
        let old = old_nodes.iter().map(String::as_str).collect::<Vec<_>>();
        let new = new_nodes.iter().map(String::as_str).collect::<Vec<_>>();
        for count in 0..=24usize {
            let acknowledgements = (0..count)
                .map(|index| format!("node_{index:02}")).collect::<Vec<_>>();
            let unique = acknowledgements.iter().map(String::as_str).collect::<Vec<_>>();
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
                expected, "distinct acknowledgements: {count}",
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
        let acks = (0..128).map(|index| if index % 2 == 0 { "a" } else { "j" })
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
