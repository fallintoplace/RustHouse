//! Detect mismatched replica snapshots before attempting promotion.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotVersion {
    pub epoch: u64,
    pub generation: u64,
    pub fingerprint: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionRelation {
    Identical,
    LocalBehind,
    LocalAhead,
    Diverged,
}

fn valid_version(version: SnapshotVersion) -> bool {
    version.epoch != 0 && version.generation != 0
}

fn valid_epoch_member(version: SnapshotVersion, epoch: u64) -> bool {
    valid_version(version) && version.epoch == epoch
}

/// Comparing versions does not establish consensus or authorize writes.
/// Different epochs or equal generations with different content must be reconciled.
pub fn compare_snapshots(local: SnapshotVersion, remote: SnapshotVersion) -> VersionRelation {
    if !valid_version(local) || !valid_epoch_member(remote, local.epoch) {
        return VersionRelation::Diverged;
    }
    match local.generation.cmp(&remote.generation) {
        std::cmp::Ordering::Less => VersionRelation::LocalBehind,
        std::cmp::Ordering::Greater => VersionRelation::LocalAhead,
        std::cmp::Ordering::Equal if local.fingerprint == remote.fingerprint => {
            VersionRelation::Identical
        }
        std::cmp::Ordering::Equal => VersionRelation::Diverged,
    }
}


/// Find the newest snapshot that was independently observed on enough
/// distinct replica members to satisfy a read quorum.
///
/// This checks metadata agreement only. It does not prove snapshot ancestry,
/// committed writes, election safety, or consensus.
pub fn newest_quorum_observed_snapshot(
    policy: crate::distributed_quorum::QuorumPolicy,
    members: &[&str],
    observations: &[(&str, SnapshotVersion)],
) -> Result<Option<SnapshotVersion>, &'static str> {
    if observations.len() > members.len() {
        return Err("too many snapshot observations for membership");
    }
    let mut acknowledging = Vec::with_capacity(observations.len());
    for (index, &(node, _)) in observations.iter().enumerate() {
        if observations[..index].iter().any(|(prior, _)| *prior == node) {
            return Err("duplicate snapshot observation");
        }
        acknowledging.push(node);
    }
    // Also validate the policy and the full member set for an empty poll.
    policy.observe(members, &acknowledging)?;
    if observations.is_empty() {
        return Ok(None);
    }
    let versions = observations.iter().map(|&(_, version)| version)
        .collect::<Vec<_>>();
    // A disagreement about the epoch or the bytes at a generation fails
    // closed, even if one candidate appears to have enough supporters.
    newest_consistent_snapshot(&versions)?;
    let mut newest = None;
    for &version in &versions {
        let support = versions.iter().filter(|&&other| other == version).count();
        if support >= policy.reads
            && newest.is_none_or(|chosen: SnapshotVersion| {
                version.generation > chosen.generation
            })
        {
            newest = Some(version);
        }
    }
    Ok(newest)
}

#[cfg(test)]
mod quorum_snapshot_observation_tests {
    use super::*;
    use crate::distributed_quorum::QuorumPolicy;

    #[test]
    fn picks_only_quorum_supported_versions() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let members = ["a", "b", "c"];
        let v4 = SnapshotVersion { epoch: 2, generation: 4, fingerprint: 44 };
        let v5 = SnapshotVersion { generation: 5, fingerprint: 55, ..v4 };
        assert_eq!(
            newest_quorum_observed_snapshot(
                policy, &members, &[("a", v4), ("b", v4), ("c", v5)]
            ).unwrap(), Some(v4)
        );
        assert_eq!(
            newest_quorum_observed_snapshot(
                policy, &members, &[("a", v5), ("b", v5), ("c", v4)]
            ).unwrap(), Some(v5)
        );
        assert_eq!(
            newest_quorum_observed_snapshot(
                policy, &members, &[("a", v4), ("b", v5)]
            ).unwrap(), None
        );
        assert_eq!(
            newest_quorum_observed_snapshot(policy, &members, &[]).unwrap(), None
        );
    }

    #[test]
    fn refuses_duplicates_unknowns_and_conflicting_histories() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let members = ["a", "b", "c"];
        let version = SnapshotVersion { epoch: 1, generation: 1, fingerprint: 1 };
        assert!(newest_quorum_observed_snapshot(
            policy, &members, &[("a", version), ("a", version)]
        ).is_err());
        assert!(newest_quorum_observed_snapshot(
            policy, &members, &[("unknown", version)]
        ).is_err());
        assert!(newest_quorum_observed_snapshot(
            policy, &members, &[("a", version), ("b", SnapshotVersion {
                fingerprint: 2, ..version
            })]
        ).is_err());
        assert!(newest_quorum_observed_snapshot(
            policy, &members, &[("a", version), ("b", SnapshotVersion {
                epoch: 2, ..version
            })]
        ).is_err());
        assert!(newest_quorum_observed_snapshot(
            policy, &["a", "a", "c"], &[]
        ).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_copied_newer_and_conflicting_snapshots() {
        let original = SnapshotVersion {
            epoch: 1,
            generation: 4,
            fingerprint: 11,
        };
        assert_eq!(
            compare_snapshots(original, original),
            VersionRelation::Identical
        );
        assert_eq!(
            compare_snapshots(
                original,
                SnapshotVersion {
                    generation: 5,
                    ..original
                }
            ),
            VersionRelation::LocalBehind
        );
        assert_eq!(
            compare_snapshots(
                original,
                SnapshotVersion {
                    epoch: 2,
                    ..original
                }
            ),
            VersionRelation::Diverged
        );
        assert_eq!(
            compare_snapshots(
                original,
                SnapshotVersion {
                    fingerprint: 12,
                    ..original
                }
            ),
            VersionRelation::Diverged
        );
    }
    #[test]
    fn reversing_snapshot_inputs_reverses_direction() {
        for local in 1..=8 {
            for remote in 1..=8 {
                let a = SnapshotVersion {
                    epoch: 2,
                    generation: local,
                    fingerprint: 10,
                };
                let b = SnapshotVersion {
                    epoch: 2,
                    generation: remote,
                    fingerprint: 10,
                };
                let forward = compare_snapshots(a, b);
                let backward = compare_snapshots(b, a);
                match forward {
                    VersionRelation::Identical => assert_eq!(backward, VersionRelation::Identical),
                    VersionRelation::LocalAhead => {
                        assert_eq!(backward, VersionRelation::LocalBehind)
                    }
                    VersionRelation::LocalBehind => {
                        assert_eq!(backward, VersionRelation::LocalAhead)
                    }
                    VersionRelation::Diverged => panic!("identical epochs cannot diverge here"),
                }
                assert_eq!(
                    compare_snapshots(a, SnapshotVersion { epoch: 3, ..b }),
                    VersionRelation::Diverged
                );
            }
        }
    }
}

/// Pick the latest generation from one metadata-consistent epoch.
/// This does not prove that different generations share a valid history or quorum.
pub fn newest_consistent_snapshot(
    versions: &[SnapshotVersion],
) -> Result<SnapshotVersion, &'static str> {
    if versions.is_empty() || versions.len() > 4096 {
        return Err("invalid snapshot candidate count");
    }
    let epoch = versions[0].epoch;
    if versions.len() <= 16 {
        // Replica groups are bounded to 16 members by the quorum policy.
        // Inspect previous generation fingerprints directly without a
        // temporary hash table for the common consensus-sized candidate set.
        let mut newest = versions[0];
        for (index, &version) in versions.iter().enumerate() {
            if !valid_epoch_member(version, epoch) {
                return Err("invalid or mismatched snapshot epoch");
            }
            if versions[..index].iter().any(|earlier| {
                earlier.generation == version.generation
                    && earlier.fingerprint != version.fingerprint
            }) {
                return Err("conflicting snapshot fingerprint");
            }
            if version.generation > newest.generation {
                newest = version;
            }
        }
        return Ok(newest);
    }
    let mut seen = std::collections::HashMap::with_capacity(versions.len());
    let mut newest = versions[0];
    for &version in versions {
        if !valid_epoch_member(version, epoch) {
            return Err("invalid or mismatched snapshot epoch");
        }
        if seen
            .insert(version.generation, version.fingerprint)
            .is_some_and(|previous| previous != version.fingerprint)
        {
            return Err("conflicting snapshot fingerprint");
        }
        if version.generation > newest.generation {
            newest = version;
        }
    }
    Ok(newest)
}

#[cfg(test)]
mod snapshot_selection_tests {
    use super::*;

    #[test]
    fn small_candidates_match_larger_group_semantics() {
        for count in [1usize, 2, 3, 8, 9, 64] {
            let versions = (1..=count)
                .rev()
                .map(|index| SnapshotVersion {
                    epoch: 7,
                    generation: index as u64,
                    fingerprint: index as u128 * 17,
                })
                .collect::<Vec<_>>();
            assert_eq!(newest_consistent_snapshot(&versions).unwrap(), versions[0]);
            let mut same_duplicate = versions.clone();
            same_duplicate.push(versions[0]);
            assert_eq!(
                newest_consistent_snapshot(&same_duplicate).unwrap(),
                versions[0]
            );
            let mut conflicting = versions.clone();
            conflicting.push(SnapshotVersion {
                fingerprint: u128::MAX,
                ..versions[0]
            });
            assert!(newest_consistent_snapshot(&conflicting).is_err());
            let mut wrong_epoch = versions.clone();
            wrong_epoch.push(SnapshotVersion { epoch: 8, ..versions[0] });
            assert!(newest_consistent_snapshot(&wrong_epoch).is_err());
        }
    }

    #[test]
    fn selects_latest_candidate_only_without_conflicts() {
        let older = SnapshotVersion { epoch: 2, generation: 3, fingerprint: 11 };
        let newer = SnapshotVersion { epoch: 2, generation: 5, fingerprint: 22 };
        assert_eq!(newest_consistent_snapshot(&[newer, older, newer]).unwrap(), newer);
        assert!(newest_consistent_snapshot(&[]).is_err());
        assert!(newest_consistent_snapshot(&[
            older,
            SnapshotVersion { epoch: 3, ..newer },
        ]).is_err());
        assert!(newest_consistent_snapshot(&[
            newer,
            SnapshotVersion { fingerprint: 33, ..newer },
        ]).is_err());
    }
}

#[cfg(test)]
mod invalid_version_comparison_tests {
    use super::*;

    #[test]
    fn invalid_versions_are_never_identical_or_ahead() {
        let good = SnapshotVersion {
            epoch: 1,
            generation: 1,
            fingerprint: 12,
        };
        for invalid in [
            SnapshotVersion { epoch: 0, ..good },
            SnapshotVersion { generation: 0, ..good },
        ] {
            assert_eq!(compare_snapshots(invalid, invalid), VersionRelation::Diverged);
            assert_eq!(compare_snapshots(good, invalid), VersionRelation::Diverged);
            assert_eq!(compare_snapshots(invalid, good), VersionRelation::Diverged);
        }
    }
}

/**
 * Select the highest snapshot version observed at the configured write
 * quorum. A minority advertisement of a newer generation is not enough.
 * This is an acknowledgement planner, not proof of a valid ancestry,
 * consensus round, or committed snapshot.
 */
pub fn highest_quorum_observed_snapshot(
    policy: crate::distributed_quorum::QuorumPolicy,
    members: &[&str],
    observations: &[(&str, SnapshotVersion)],
) -> Result<Option<SnapshotVersion>, &'static str> {
    // This validates replica IDs, unique membership and a safe quorum policy.
    policy.observe(members, &[])?;
    if observations.len() > members.len() {
        return Err("too many observed snapshot versions");
    }
    let mut observed_nodes = 0u16;
    let mut epoch = None;
    // A quorum policy supports at most sixteen replicas. Grouping their
    // snapshot identities on the stack avoids a tree allocation and sorting.
    let mut groups = [(0u64, 0u128, 0usize); 16];
    let mut group_count = 0usize;
    for &(member, version) in observations {
        let index = members.iter().position(|&known| member == known)
            .ok_or("snapshot observation from unknown replica")?;
        let bit = 1u16 << index;
        if observed_nodes & bit != 0 {
            return Err("duplicate snapshot observation");
        }
        observed_nodes |= bit;
        if !valid_version(version) {
            return Err("invalid snapshot version");
        }
        if epoch.is_some_and(|previous| previous != version.epoch) {
            return Err("mismatched snapshot epoch");
        }
        epoch = Some(version.epoch);
        if let Some(group) = groups[..group_count].iter_mut()
            .find(|group| group.0 == version.generation)
        {
            if group.1 != version.fingerprint {
                return Err("conflicting snapshot fingerprint");
            }
            group.2 += 1;
        } else {
            groups[group_count] = (version.generation, version.fingerprint, 1);
            group_count += 1;
        }
    }
    Ok(groups[..group_count]
        .iter()
        .filter(|group| group.2 >= policy.writes)
        .max_by_key(|group| group.0)
        .map(|&(generation, fingerprint, _)| SnapshotVersion {
            epoch: epoch.expect("nonempty groups have an epoch"),
            generation,
            fingerprint,
        }))
}

#[cfg(test)]
mod quorum_snapshot_selection_tests {
    use super::*;
    use crate::distributed_quorum::QuorumPolicy;

    #[test]
    fn minority_newer_generation_does_not_displace_quorum_snapshot() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let old = SnapshotVersion { epoch: 1, generation: 9, fingerprint: 99 };
        let new = SnapshotVersion { epoch: 1, generation: 10, fingerprint: 101 };
        let members = ["a", "b", "c"];
        assert_eq!(
            highest_quorum_observed_snapshot(policy, &members,
                &[("a", new), ("b", old), ("c", old)]).unwrap(),
            Some(old),
        );
        assert_eq!(
            highest_quorum_observed_snapshot(policy, &members,
                &[("a", new), ("b", new), ("c", old)]).unwrap(),
            Some(new),
        );
        assert_eq!(
            highest_quorum_observed_snapshot(policy, &members, &[("a", new)]).unwrap(),
            None,
        );
    }

    #[test]
    fn rejects_conflicts_duplicate_votes_foreign_epochs_and_nodes() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let members = ["a", "b", "c"];
        let version = SnapshotVersion { epoch: 5, generation: 7, fingerprint: 19 };
        assert!(highest_quorum_observed_snapshot(policy, &members, &[
            ("a", version), ("b", SnapshotVersion { fingerprint: 20, ..version })
        ]).is_err());
        assert!(highest_quorum_observed_snapshot(policy, &members, &[
            ("a", version), ("a", version)
        ]).is_err());
        assert!(highest_quorum_observed_snapshot(policy, &members, &[
            ("a", version), ("b", SnapshotVersion { epoch: 6, ..version })
        ]).is_err());
        assert!(highest_quorum_observed_snapshot(policy, &members, &[
            ("unknown", version)
        ]).is_err());
        assert!(highest_quorum_observed_snapshot(policy, &members, &[
            ("a", SnapshotVersion { generation: 0, ..version })
        ]).is_err());
        assert!(highest_quorum_observed_snapshot(policy, &["a", "a", "c"], &[]).is_err());
    }

    #[test]
    fn many_replica_quorum_requires_distinct_observations() {
        let policy = QuorumPolicy::new(16, 9, 9).unwrap();
        let names = (0..16).map(|i| format!("node_{i}")).collect::<Vec<_>>();
        let members = names.iter().map(String::as_str).collect::<Vec<_>>();
        let version = SnapshotVersion { epoch: 2, generation: 7, fingerprint: 99 };
        let observed = members.iter().take(9).map(|&id| (id, version)).collect::<Vec<_>>();
        assert_eq!(highest_quorum_observed_snapshot(policy, &members, &observed).unwrap(), Some(version));
        assert_eq!(highest_quorum_observed_snapshot(policy, &members, &observed[..8]).unwrap(), None);
    }
}

#[cfg(test)]
mod stack_quorum_snapshot_tests {
    use super::*;
    use crate::distributed_quorum::QuorumPolicy;
    use std::collections::BTreeMap;

    #[test]
    fn stack_groups_match_reference_for_all_quorum_sizes() {
        for replicas in 1..=16usize {
            let names = (0..replicas).map(|i| format!("node_{i}")).collect::<Vec<_>>();
            let members = names.iter().map(String::as_str).collect::<Vec<_>>();
            for writes in (replicas / 2 + 1)..=replicas {
                let policy = QuorumPolicy::new(replicas, replicas - writes + 1, writes).unwrap();
                for shift in 0..replicas {
                    let versions = (0..replicas)
                        .map(|i| {
                            let generation = 1 + ((i + shift) % 7) as u64;
                            (members[i], SnapshotVersion {
                                epoch: 3, generation, fingerprint: u128::from(generation * 19),
                            })
                        })
                        .collect::<Vec<_>>();
                    let mut reference = BTreeMap::<u64, usize>::new();
                    for &(_, version) in &versions {
                        *reference.entry(version.generation).or_default() += 1;
                    }
                    let expected = reference.iter().rev()
                        .find(|entry| *entry.1 >= writes)
                        .map(|(&generation, _)| SnapshotVersion {
                            epoch: 3, generation, fingerprint: u128::from(generation * 19),
                        });
                    assert_eq!(
                        highest_quorum_observed_snapshot(policy, &members, &versions).unwrap(),
                        expected,
                        "replicas={replicas} writes={writes} shift={shift}",
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_conflicting_identity_even_without_a_quorum() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let members = ["a", "b", "c"];
        let version = SnapshotVersion { epoch: 1, generation: 10, fingerprint: 100 };
        assert!(highest_quorum_observed_snapshot(policy, &members, &[
            ("a", version), ("b", SnapshotVersion { fingerprint: 101, ..version }),
        ]).is_err());
        assert!(highest_quorum_observed_snapshot(policy, &members, &[
            ("a", version), ("a", version),
        ]).is_err());
        assert_eq!(highest_quorum_observed_snapshot(policy, &members, &[]).unwrap(), None);
    }
}

#[cfg(test)]
mod consensus_sized_snapshot_selection_tests {
    use super::*;
    use std::collections::HashMap;

    fn hash_reference(versions: &[SnapshotVersion]) -> Result<SnapshotVersion, &'static str> {
        if versions.is_empty() || versions.len() > 4096 {
            return Err("invalid snapshot candidate count");
        }
        let epoch = versions[0].epoch;
        let mut seen = HashMap::new();
        let mut latest = versions[0];
        for &version in versions {
            if !valid_epoch_member(version, epoch) {
                return Err("invalid or mismatched snapshot epoch");
            }
            if seen.insert(version.generation, version.fingerprint)
                .is_some_and(|previous| previous != version.fingerprint)
            {
                return Err("conflicting snapshot fingerprint");
            }
            if version.generation > latest.generation {
                latest = version;
            }
        }
        Ok(latest)
    }

    #[test]
    fn candidate_limits_preserve_exact_large_path_semantics() {
        for count in [1, 2, 8, 9, 15, 16, 17, 128] {
            let versions = (0..count).map(|index| SnapshotVersion {
                epoch: 4,
                generation: (index % 11 + 1) as u64,
                fingerprint: ((index % 11) * 71 + 3) as u128,
            }).collect::<Vec<_>>();
            assert_eq!(newest_consistent_snapshot(&versions), hash_reference(&versions));
            let mut conflicting = versions.clone();
            conflicting.push(SnapshotVersion {
                fingerprint: u128::MAX,
                ..versions[0]
            });
            assert_eq!(
                newest_consistent_snapshot(&conflicting),
                hash_reference(&conflicting)
            );
            let mut wrong_epoch = versions;
            wrong_epoch.push(SnapshotVersion { epoch: 9, ..wrong_epoch[0] });
            assert_eq!(
                newest_consistent_snapshot(&wrong_epoch),
                hash_reference(&wrong_epoch)
            );
        }
    }
}


/// Diagnostics about reports relative to a quorum-observed snapshot.
///
/// A newer minority observation is *not* proof of a committed generation,
/// and a lagging observation does not establish common ancestry or authorize
/// overwrite. This is read-only metadata classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuorumSnapshotLag {
    pub confirmed: Option<SnapshotVersion>,
    pub behind: Vec<String>,
    pub ahead_without_quorum: Vec<String>,
    pub unobserved: Vec<String>,
}

/// Classify a bounded replica poll without treating a minority's newer
/// generation as committed or assuming that an unresponsive node is stale.
pub fn classify_quorum_snapshot_lag(
    policy: crate::distributed_quorum::QuorumPolicy,
    members: &[&str],
    observations: &[(&str, SnapshotVersion)],
) -> Result<QuorumSnapshotLag, &'static str> {
    let confirmed = newest_quorum_observed_snapshot(policy, members, observations)?;
    let mut report = QuorumSnapshotLag {
        confirmed,
        behind: Vec::new(),
        ahead_without_quorum: Vec::new(),
        unobserved: Vec::new(),
    };
    for &member in members {
        match observations.iter().find(|&&(node, _)| node == member) {
            None => report.unobserved.push(member.to_owned()),
            Some(&(_, version)) => {
                if let Some(chosen) = confirmed {
                    if version.generation < chosen.generation {
                        report.behind.push(member.to_owned());
                    } else if version.generation > chosen.generation {
                        report.ahead_without_quorum.push(member.to_owned());
                    }
                }
            }
        }
    }
    report.behind.sort_unstable();
    report.ahead_without_quorum.sort_unstable();
    report.unobserved.sort_unstable();
    Ok(report)
}

#[cfg(test)]
mod quorum_snapshot_lag_classification_tests {
    use super::*;
    use crate::distributed_quorum::QuorumPolicy;

    #[test]
    fn minority_newer_versions_are_not_marked_as_committed() {
        let policy = QuorumPolicy::new(5, 3, 3).unwrap();
        let v4 = SnapshotVersion { epoch: 7, generation: 4, fingerprint: 44 };
        let v3 = SnapshotVersion { generation: 3, fingerprint: 33, ..v4 };
        let v5 = SnapshotVersion { generation: 5, fingerprint: 55, ..v4 };
        let members = ["e", "d", "c", "b", "a"];
        let report = classify_quorum_snapshot_lag(
            policy, &members,
            &[("a", v4), ("b", v4), ("c", v4), ("d", v3), ("e", v5)],
        ).unwrap();
        assert_eq!(report.confirmed, Some(v4));
        assert_eq!(report.behind, ["d"]);
        assert_eq!(report.ahead_without_quorum, ["e"]);
        assert!(report.unobserved.is_empty());
        let report = classify_quorum_snapshot_lag(
            policy, &members,
            &[("a", v4), ("b", v4), ("c", v4), ("d", v3)],
        ).unwrap();
        assert_eq!(report.confirmed, Some(v4));
        assert_eq!(report.behind, ["d"]);
        assert_eq!(report.unobserved, ["e"]);
    }

    #[test]
    fn no_quorum_never_labels_a_replica_safe_to_repair() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let v1 = SnapshotVersion { epoch: 1, generation: 1, fingerprint: 10 };
        let v2 = SnapshotVersion { generation: 2, fingerprint: 20, ..v1 };
        let report = classify_quorum_snapshot_lag(
            policy, &["a", "b", "c"], &[("a", v1), ("b", v2)],
        ).unwrap();
        assert_eq!(report.confirmed, None);
        assert!(report.behind.is_empty());
        assert!(report.ahead_without_quorum.is_empty());
        assert_eq!(report.unobserved, ["c"]);
    }

    #[test]
    fn conflicting_history_unknown_nodes_and_duplicate_reports_fail_closed() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let v1 = SnapshotVersion { epoch: 1, generation: 1, fingerprint: 10 };
        for reports in [
            vec![("a", v1), ("b", SnapshotVersion { fingerprint: 11, ..v1 })],
            vec![("a", v1), ("a", v1)],
            vec![("missing", v1)],
        ] {
            assert!(classify_quorum_snapshot_lag(
                policy, &["a", "b", "c"], &reports,
            ).is_err());
        }
    }
}
