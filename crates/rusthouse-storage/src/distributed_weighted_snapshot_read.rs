//! Quorum-supported snapshot observation, not a proof of commit or ancestry.
use crate::distributed_lineage::SnapshotVersion;
use crate::distributed_membership::{Member, validate_membership};
use crate::distributed_weighted_quorum::WeightedQuorumPolicy;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotObservation<'a> {
    pub node: &'a str,
    pub version: SnapshotVersion,
}

/// An observed read quorum may be stale. Callers still require a separate
/// authoritative commit certificate and snapshot ancestry checks.
pub fn newest_weighted_observed_snapshot(
    members: &[Member],
    policy: WeightedQuorumPolicy,
    observations: &[SnapshotObservation<'_>],
) -> Result<Option<SnapshotVersion>, &'static str> {
    validate_membership(members)?;
    WeightedQuorumPolicy::new(members, policy.reads, policy.writes)?;
    if observations.len() > members.len() {
        return Err("too many weighted snapshot observations");
    }
    let mut seen = HashSet::new();
    let mut epoch = None;
    let mut fingerprints = HashMap::<u64, u128>::new();
    let mut totals = HashMap::<(u64, u128), u64>::new();
    for observation in observations {
        let member = members.iter().find(|member| member.node == observation.node)
            .ok_or("snapshot observation from unknown member")?;
        if !seen.insert(observation.node) {
            return Err("duplicate weighted snapshot observation");
        }
        let version = observation.version;
        if version.epoch == 0 || version.generation == 0
            || epoch.is_some_and(|expected| expected != version.epoch)
        {
            return Err("invalid or conflicting snapshot epoch");
        }
        epoch = Some(version.epoch);
        if let Some(&existing) = fingerprints.get(&version.generation) {
            if existing != version.fingerprint {
                return Err("conflicting snapshot generation fingerprints");
            }
        } else {
            fingerprints.insert(version.generation, version.fingerprint);
        }
        *totals.entry((version.generation, version.fingerprint)).or_insert(0) +=
            u64::from(member.weight);
    }
    Ok(totals.into_iter()
        .filter(|(_, weight)| *weight >= policy.reads)
        .max_by_key(|((generation, _), _)| *generation)
        .map(|((generation, fingerprint), _)| SnapshotVersion {
            epoch: epoch.expect("nonempty observation groups"),
            generation,
            fingerprint,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members() -> Vec<Member> {
        [("a", 4), ("b", 2), ("c", 1)].into_iter()
            .map(|(node, weight)| Member {
                node: node.into(), zone: "zone".into(), weight,
            }).collect()
    }
    fn v(generation: u64, fingerprint: u128) -> SnapshotVersion {
        SnapshotVersion { epoch: 1, generation, fingerprint }
    }

    #[test]
    fn heavy_voter_meets_read_threshold_alone() {
        let group = members();
        let policy = WeightedQuorumPolicy::new(&group, 3, 5).unwrap();
        let observed = [
            SnapshotObservation { node: "a", version: v(6, 66) },
            SnapshotObservation { node: "b", version: v(7, 77) },
            SnapshotObservation { node: "c", version: v(7, 77) },
        ];
        assert_eq!(newest_weighted_observed_snapshot(&group, policy, &observed).unwrap(), Some(v(7, 77)));
    }

    #[test]
    fn fingerprint_conflict_is_not_silently_voted_away() {
        let group = members();
        let policy = WeightedQuorumPolicy::new(&group, 3, 5).unwrap();
        assert!(newest_weighted_observed_snapshot(&group, policy, &[
            SnapshotObservation { node: "a", version: v(7, 10) },
            SnapshotObservation { node: "b", version: v(7, 11) },
        ]).is_err());
        assert!(newest_weighted_observed_snapshot(&group, policy, &[
            SnapshotObservation { node: "a", version: v(7, 10) },
            SnapshotObservation { node: "a", version: v(7, 10) },
        ]).is_err());
    }
}
