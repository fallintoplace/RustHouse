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

/// Comparing versions does not establish consensus or authorize writes.
/// Different epochs or equal generations with different content must be reconciled.
pub fn compare_snapshots(local: SnapshotVersion, remote: SnapshotVersion) -> VersionRelation {
    if !valid_version(local)
        || !valid_version(remote)
        || local.epoch != remote.epoch
    {
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
    if versions.len() <= 8 {
        // The normal replica set is small. Validate generations in-place
        // without constructing a temporary fingerprint hash map.
        let mut newest = versions[0];
        for (index, &version) in versions.iter().enumerate() {
            if !valid_version(version) || version.epoch != epoch {
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
        if !valid_version(version) || version.epoch != epoch {
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
            wrong_epoch[count - 1].epoch = 8;
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
