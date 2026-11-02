//! Weighted read/write quorum certificates for explicit replica membership.
//! This is a deterministic safety checker, not a consensus protocol.

use crate::distributed_membership::{Member, validate_membership};

/// Quorum thresholds measured in member weights, not member counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightedQuorumPolicy {
    pub reads: u64,
    pub writes: u64,
}

/// Distinct member acknowledgements observed for one operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightedQuorumObservation {
    pub voters: usize,
    pub acknowledged_weight: u64,
    pub can_read: bool,
    pub can_write: bool,
}

fn total_weight(members: &[Member]) -> Result<u64, &'static str> {
    validate_membership(members)?;
    members.iter().try_fold(0u64, |sum, member| {
        sum.checked_add(u64::from(member.weight))
            .ok_or("replica weight overflow")
    })
}

impl WeightedQuorumPolicy {
    /// Require read/write and write/write intersections by total voting weight.
    ///
    /// Every two sets reaching their respective thresholds intersect because
    /// reads + writes > total and 2 * writes > total. These conditions do not
    /// establish durable writes, freshness, elections, or consensus.
    pub fn new(members: &[Member], reads: u64, writes: u64) -> Result<Self, &'static str> {
        let total = total_weight(members)?;
        if reads == 0
            || writes == 0
            || reads > total
            || writes > total
            || reads <= total - writes
            || writes <= total / 2
        {
            return Err("unsafe weighted quorum policy");
        }
        Ok(Self { reads, writes })
    }

    /// Count each known member at most once, even if a transport retries an ACK.
    ///
    /// Unknown member IDs and invalid membership fail the entire observation.
    pub fn observe(
        self,
        members: &[Member],
        acknowledgements: &[&str],
    ) -> Result<WeightedQuorumObservation, &'static str> {
        Self::new(members, self.reads, self.writes)?;
        if acknowledgements.len() > 256 {
            return Err("too many replica acknowledgements");
        }
        let mut seen = 0u64;
        let mut acknowledged_weight = 0u64;
        let mut voters = 0usize;
        for &name in acknowledgements {
            let index = members
                .iter()
                .position(|member| member.node == name)
                .ok_or("acknowledgement from unknown replica")?;
            let bit = 1u64 << index;
            if seen & bit == 0 {
                seen |= bit;
                acknowledged_weight += u64::from(members[index].weight);
                voters += 1;
            }
        }
        Ok(WeightedQuorumObservation {
            voters,
            acknowledged_weight,
            can_read: acknowledged_weight >= self.reads,
            can_write: acknowledged_weight >= self.writes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members() -> Vec<Member> {
        [
            ("a", "east", 4),
            ("b", "west", 2),
            ("c", "central", 1),
        ]
        .into_iter()
        .map(|(node, zone, weight)| Member {
            node: node.into(),
            zone: zone.into(),
            weight,
        })
        .collect()
    }

    #[test]
    fn asymmetric_weights_and_retried_votes() {
        let members = members();
        let policy = WeightedQuorumPolicy::new(&members, 3, 5).unwrap();
        let read = policy.observe(&members, &["b", "c", "b"]).unwrap();
        assert_eq!(read.voters, 2);
        assert_eq!(read.acknowledged_weight, 3);
        assert!(read.can_read);
        assert!(!read.can_write);
        let write = policy.observe(&members, &["a", "b"]).unwrap();
        assert_eq!(write.acknowledged_weight, 6);
        assert!(write.can_read && write.can_write);
        assert!(!policy.observe(&members, &[]).unwrap().can_read);
    }

    #[test]
    fn intersections_are_enforced_at_exact_boundaries() {
        let members = members();
        assert!(WeightedQuorumPolicy::new(&members, 3, 4).is_err());
        assert!(WeightedQuorumPolicy::new(&members, 2, 5).is_err());
        assert!(WeightedQuorumPolicy::new(&members, 3, 5).is_ok());
        assert!(WeightedQuorumPolicy::new(&members, 0, 7).is_err());
        assert!(WeightedQuorumPolicy::new(&members, 8, 7).is_err());
    }

    #[test]
    fn cannot_count_unknown_or_duplicate_members() {
        let members = members();
        let policy = WeightedQuorumPolicy::new(&members, 3, 5).unwrap();
        assert!(policy.observe(&members, &["a", "other"]).is_err());
        let mut bad = members.clone();
        bad[1].node = "a".into();
        assert!(policy.observe(&bad, &["a"]).is_err());
        let mut bad = members;
        bad[0].weight = 0;
        assert!(WeightedQuorumPolicy::new(&bad, 3, 5).is_err());
    }

    #[test]
    fn all_small_weight_distributions_have_intersecting_quorums() {
        for a in 1..=4 {
            for b in 1..=4 {
                for c in 1..=4 {
                    let mut members = members();
                    members[0].weight = a;
                    members[1].weight = b;
                    members[2].weight = c;
                    let total = u64::from(a + b + c);
                    for reads in 1..=total {
                        for writes in 1..=total {
                            let Ok(policy) = WeightedQuorumPolicy::new(&members, reads, writes)
                            else {
                                continue;
                            };
                            for x in 0..8u8 {
                                for y in 0..8u8 {
                                    let voters_x = [0, 1, 2]
                                        .into_iter()
                                        .filter(|&i| (x >> i) & 1 != 0)
                                        .map(|i| members[i].node.as_str())
                                        .collect::<Vec<_>>();
                                    let voters_y = [0, 1, 2]
                                        .into_iter()
                                        .filter(|&i| (y >> i) & 1 != 0)
                                        .map(|i| members[i].node.as_str())
                                        .collect::<Vec<_>>();
                                    let rx = policy.observe(&members, &voters_x).unwrap();
                                    let wy = policy.observe(&members, &voters_y).unwrap();
                                    if rx.can_read && wy.can_write {
                                        assert_ne!(x & y, 0);
                                    }
                                    if rx.can_write && wy.can_write {
                                        assert_ne!(x & y, 0);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}


/// A joint-config acknowledgement must satisfy both the departing and the
/// incoming membership. Neither configuration may commit by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JointWeightedQuorumObservation {
    pub old: WeightedQuorumObservation,
    pub new: WeightedQuorumObservation,
    pub can_read: bool,
    pub can_write: bool,
}

/// Evaluate one acknowledgement certificate during an explicit membership
/// transition. Members added to one configuration are ignored by the other.
///
/// A node present in both configurations may have different voting weights.
/// This function validates every acknowledgement against the union and never
/// counts a duplicate acknowledgement twice in either configuration.
pub fn observe_joint_weighted_quorum(
    old_members: &[Member],
    old_policy: WeightedQuorumPolicy,
    new_members: &[Member],
    new_policy: WeightedQuorumPolicy,
    acknowledgements: &[&str],
) -> Result<JointWeightedQuorumObservation, &'static str> {
    WeightedQuorumPolicy::new(old_members, old_policy.reads, old_policy.writes)?;
    WeightedQuorumPolicy::new(new_members, new_policy.reads, new_policy.writes)?;
    if acknowledgements.len() > 256 {
        return Err("too many joint quorum acknowledgements");
    }

    let mut old_acks = Vec::with_capacity(acknowledgements.len());
    let mut new_acks = Vec::with_capacity(acknowledgements.len());
    for &node in acknowledgements {
        let in_old = old_members.iter().any(|member| member.node == node);
        let in_new = new_members.iter().any(|member| member.node == node);
        if !in_old && !in_new {
            return Err("acknowledgement from unknown joint quorum member");
        }
        if in_old {
            old_acks.push(node);
        }
        if in_new {
            new_acks.push(node);
        }
    }

    let old = old_policy.observe(old_members, &old_acks)?;
    let new = new_policy.observe(new_members, &new_acks)?;
    Ok(JointWeightedQuorumObservation {
        can_read: old.can_read && new.can_read,
        can_write: old.can_write && new.can_write,
        old,
        new,
    })
}

#[cfg(test)]
mod joint_quorum_tests {
    use super::*;

    fn member(node: &str, weight: u16) -> Member {
        Member {
            node: node.into(),
            zone: "zone".into(),
            weight,
        }
    }

    #[test]
    fn requires_both_configurations_to_approve_a_transition() {
        let before = [member("a", 2), member("b", 2), member("c", 1)];
        let after = [member("b", 2), member("c", 1), member("d", 2)];
        let old = WeightedQuorumPolicy::new(&before, 3, 3).unwrap();
        let new = WeightedQuorumPolicy::new(&after, 3, 3).unwrap();

        let only_old =
            observe_joint_weighted_quorum(&before, old, &after, new, &["a", "b"]).unwrap();
        assert!(only_old.old.can_write);
        assert!(!only_old.new.can_write);
        assert!(!only_old.can_write);

        let only_new =
            observe_joint_weighted_quorum(&before, old, &after, new, &["b", "d"]).unwrap();
        assert!(!only_new.old.can_write);
        assert!(only_new.new.can_write);
        assert!(!only_new.can_write);

        let joint = observe_joint_weighted_quorum(
            &before,
            old,
            &after,
            new,
            &["d", "b", "a", "d"],
        )
        .unwrap();
        assert_eq!(joint.old.acknowledged_weight, 4);
        assert_eq!(joint.new.acknowledged_weight, 4);
        assert!(joint.can_read && joint.can_write);
    }

    #[test]
    fn changed_weights_and_invalid_certificates_fail_closed() {
        let before = [member("a", 3), member("b", 1)];
        let after = [member("a", 1), member("b", 3)];
        let old = WeightedQuorumPolicy::new(&before, 2, 3).unwrap();
        let new = WeightedQuorumPolicy::new(&after, 2, 3).unwrap();
        let a = observe_joint_weighted_quorum(&before, old, &after, new, &["a"]).unwrap();
        assert_eq!(a.old.acknowledged_weight, 3);
        assert_eq!(a.new.acknowledged_weight, 1);
        assert!(!a.can_write);
        assert!(observe_joint_weighted_quorum(
            &before,
            old,
            &after,
            new,
            &["a", "unknown"]
        )
        .is_err());
        let mut invalid = after.to_vec();
        invalid[1].node = "a".into();
        assert!(observe_joint_weighted_quorum(&before, old, &invalid, new, &[]).is_err());
    }
}
