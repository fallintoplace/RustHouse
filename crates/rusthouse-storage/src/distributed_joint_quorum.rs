//! Check quorum intersection across a planned membership replacement.
//! This is an offline safety check, not joint consensus or an election protocol.
use crate::distributed_membership::{Member, validate_membership};
use crate::distributed_weighted_quorum::WeightedQuorumPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JointQuorumSafety {
    pub writes_intersect: bool,
    pub old_reads_intersect_new_writes: bool,
    pub new_reads_intersect_old_writes: bool,
}

impl JointQuorumSafety {
    pub fn safe(self) -> bool {
        self.writes_intersect
            && self.old_reads_intersect_new_writes
            && self.new_reads_intersect_old_writes
    }
}

/// Decide exact weighted-quorum intersection, including asymmetric weights.
///
/// A counterexample assigns each shared member to at most one quorum. Members
/// exclusive to either configuration can always participate in that side.
/// Enumerating the shared assignments is exact for the bounded transition.
fn intersect(
    shared: &[(u64, u64)],
    old_only: u64,
    new_only: u64,
    old_threshold: u64,
    new_threshold: u64,
) -> bool {
    for mask in 0..(1u64 << shared.len()) {
        let mut old_weight = old_only;
        let mut new_weight = new_only;
        for (index, &(old, new)) in shared.iter().enumerate() {
            if mask & (1u64 << index) == 0 {
                old_weight += old;
            } else {
                new_weight += new;
            }
        }
        if old_weight >= old_threshold && new_weight >= new_threshold {
            return false;
        }
    }
    true
}

/// Reject a membership change whenever a qualifying old and new quorum can
/// be formed from disjoint physical members. This is a necessary safety check
/// for reconfiguration; it does not make a reconfiguration atomic or durable.
///
/// Limit the overlap to 20 members so exhaustive checking is bounded.
pub fn validate_joint_quorums(
    before: &[Member],
    old: WeightedQuorumPolicy,
    after: &[Member],
    new: WeightedQuorumPolicy,
) -> Result<JointQuorumSafety, &'static str> {
    validate_membership(before)?;
    validate_membership(after)?;
    WeightedQuorumPolicy::new(before, old.reads, old.writes)?;
    WeightedQuorumPolicy::new(after, new.reads, new.writes)?;

    let mut shared = Vec::new();
    let mut before_only = 0u64;
    for member in before {
        if let Some(other) = after.iter().find(|other| other.node == member.node) {
            shared.push((u64::from(member.weight), u64::from(other.weight)));
        } else {
            before_only += u64::from(member.weight);
        }
    }
    if shared.len() > 20 {
        return Err("joint quorum overlap exceeds exhaustive safety limit");
    }
    let after_only = after
        .iter()
        .filter(|member| !before.iter().any(|old| old.node == member.node))
        .map(|member| u64::from(member.weight))
        .sum();
    Ok(JointQuorumSafety {
        writes_intersect: intersect(&shared, before_only, after_only, old.writes, new.writes),
        old_reads_intersect_new_writes: intersect(
            &shared, before_only, after_only, old.reads, new.writes,
        ),
        new_reads_intersect_old_writes: intersect(
            &shared.iter().map(|&(a, b)| (b, a)).collect::<Vec<_>>(),
            after_only, before_only, new.reads, old.writes,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(names: &[(&str, u16)]) -> Vec<Member> {
        names.iter().map(|&(node, weight)| Member {
            node: node.to_owned(), zone: "z".into(), weight
        }).collect()
    }

    #[test]
    fn identical_majorities_intersect_in_all_directions() {
        let group = members(&[("a", 1), ("b", 1), ("c", 1)]);
        let quorum = WeightedQuorumPolicy::new(&group, 2, 2).unwrap();
        assert!(validate_joint_quorums(&group, quorum, &group, quorum).unwrap().safe());
    }

    #[test]
    fn replacing_a_voter_breaks_cross_generation_safety() {
        let old = members(&[("a", 1), ("b", 1), ("c", 1)]);
        let new = members(&[("b", 1), ("c", 1), ("d", 1)]);
        let policy = WeightedQuorumPolicy::new(&old, 2, 2).unwrap();
        assert!(!validate_joint_quorums(&old, policy, &new, policy).unwrap().safe());
    }

    #[test]
    fn weighted_counterexample_and_invalid_policies() {
        let old = members(&[("a", 4), ("b", 2), ("c", 1)]);
        let new = members(&[("b", 2), ("c", 1), ("d", 4)]);
        let policy = WeightedQuorumPolicy::new(&old, 3, 5).unwrap();
        let result = validate_joint_quorums(&old, policy, &new, policy).unwrap();
        assert!(!result.writes_intersect);
        assert!(validate_joint_quorums(
            &old, WeightedQuorumPolicy { reads: 1, writes: 1 }, &new, policy
        ).is_err());
    }

    #[test]
    fn exclusive_members_are_not_double_counted() {
        let old = members(&[("a", 1), ("b", 1), ("c", 1)]);
        let new = members(&[("d", 1), ("e", 1), ("f", 1)]);
        let policy = WeightedQuorumPolicy::new(&old, 2, 2).unwrap();
        let result = validate_joint_quorums(&old, policy, &new, policy).unwrap();
        assert_eq!(result.writes_intersect, false);
        assert!(!result.safe());
    }
}
