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

    /// Determine availability from actual replica IDs instead of trusting a
    /// caller-supplied acknowledgement count. Duplicate replies never count
    /// as more than one vote. This is planning, not a consensus protocol.
    pub fn observe(
        self,
        members: &[&str],
        acknowledgements: &[&str],
    ) -> Result<QuorumObservation, &'static str> {
        Self::new(self.replicas, self.reads, self.writes)?;
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
            can_read: self.can_read(distinct_acks),
            can_write: self.can_write(distinct_acks),
        })
    }

    fn valid_ack_count(self, unique_acks: usize) -> bool {
        Self::new(self.replicas, self.reads, self.writes).is_ok()
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
