//! Weighted joint-membership ACK verification for a proposed snapshot.
//! A quorum certificate is metadata, not a consensus implementation.
use crate::distributed_lineage::SnapshotVersion;
use crate::distributed_membership::{Member, validate_membership};
use crate::distributed_weighted_quorum::WeightedQuorumPolicy;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotAck<'a> {
    pub node: &'a str,
    pub snapshot: SnapshotVersion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JointCommitObservation {
    pub old_weight: u64,
    pub new_weight: u64,
    pub old_quorum: bool,
    pub new_quorum: bool,
    pub can_commit: bool,
}

/// Verify every accepted acknowledgement refers to the exact proposed
/// immutable generation and fingerprint, not only the generation number.
/// Shared members contribute their own configured weight in each membership.
pub fn observe_joint_commit(
    old_members: &[Member],
    old_policy: WeightedQuorumPolicy,
    new_members: &[Member],
    new_policy: WeightedQuorumPolicy,
    proposed: SnapshotVersion,
    acknowledgements: &[SnapshotAck<'_>],
) -> Result<JointCommitObservation, &'static str> {
    validate_membership(old_members)?;
    validate_membership(new_members)?;
    WeightedQuorumPolicy::new(old_members, old_policy.reads, old_policy.writes)?;
    WeightedQuorumPolicy::new(new_members, new_policy.reads, new_policy.writes)?;
    if proposed.epoch == 0 || proposed.generation == 0
        || acknowledgements.len() > old_members.len() + new_members.len()
    {
        return Err("invalid proposed snapshot or joint ACK count");
    }
    let mut old_weight = 0u64;
    let mut new_weight = 0u64;
    let mut seen = HashSet::new();
    for ack in acknowledgements {
        if ack.snapshot != proposed {
            return Err("joint ACK references conflicting snapshot");
        }
        if !seen.insert(ack.node) {
            return Err("duplicate joint commit acknowledgement");
        }
        let old = old_members.iter().find(|m| m.node == ack.node);
        let new = new_members.iter().find(|m| m.node == ack.node);
        if old.is_none() && new.is_none() {
            return Err("joint ACK from unrecognized replica");
        }
        if let Some(member) = old { old_weight += u64::from(member.weight); }
        if let Some(member) = new { new_weight += u64::from(member.weight); }
    }
    let old_quorum = old_weight >= old_policy.writes;
    let new_quorum = new_weight >= new_policy.writes;
    Ok(JointCommitObservation {
        old_weight, new_weight, old_quorum, new_quorum,
        can_commit: old_quorum && new_quorum,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn members(nodes: &[(&str, u16)]) -> Vec<Member> {
        nodes.iter().map(|&(node, weight)| Member {
            node: node.into(), zone: "z".into(), weight
        }).collect()
    }
    fn version(hash: u128) -> SnapshotVersion {
        SnapshotVersion { epoch: 1, generation: 7, fingerprint: hash }
    }

    #[test]
    fn both_voting_configurations_must_acknowledge() {
        let old = members(&[("a", 1), ("b", 1), ("c", 1)]);
        let new = members(&[("b", 1), ("c", 1), ("d", 1)]);
        let policy = WeightedQuorumPolicy::new(&old, 2, 2).unwrap();
        let ack = |node| SnapshotAck { node, snapshot: version(99) };
        let partial = observe_joint_commit(&old, policy, &new, policy,
            version(99), &[ack("a"), ack("b")]).unwrap();
        assert!(partial.old_quorum);
        assert!(!partial.new_quorum);
        assert!(!partial.can_commit);
        let complete = observe_joint_commit(&old, policy, &new, policy,
            version(99), &[ack("b"), ack("c")]).unwrap();
        assert!(complete.can_commit);
        assert_eq!((complete.old_weight, complete.new_weight), (2, 2));
    }

    #[test]
    fn wrong_fingerprint_or_duplicate_ack_is_rejected() {
        let old = members(&[("a", 2), ("b", 1)]);
        let policy = WeightedQuorumPolicy::new(&old, 2, 2).unwrap();
        assert!(observe_joint_commit(&old, policy, &old, policy, version(5),
            &[SnapshotAck { node: "a", snapshot: version(6) }]).is_err());
        assert!(observe_joint_commit(&old, policy, &old, policy, version(5),
            &[SnapshotAck { node: "a", snapshot: version(5) },
              SnapshotAck { node: "a", snapshot: version(5) }]).is_err());
    }
}
