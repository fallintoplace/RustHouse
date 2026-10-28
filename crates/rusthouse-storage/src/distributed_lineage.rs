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

/// Comparing versions does not establish consensus or authorize writes.
/// Different epochs or equal generations with different content must be reconciled.
pub fn compare_snapshots(local: SnapshotVersion, remote: SnapshotVersion) -> VersionRelation {
    if local.epoch != remote.epoch {
        return VersionRelation::Diverged;
    }
    if local.generation == remote.generation {
        return if local.fingerprint == remote.fingerprint {
            VersionRelation::Identical
        } else {
            VersionRelation::Diverged
        };
    }
    if local.generation < remote.generation {
        VersionRelation::LocalBehind
    } else {
        VersionRelation::LocalAhead
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
