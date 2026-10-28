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
        let original = SnapshotVersion { epoch: 1, generation: 4, fingerprint: 11 };
        assert_eq!(compare_snapshots(original, original), VersionRelation::Identical);
        assert_eq!(
            compare_snapshots(original, SnapshotVersion { generation: 5, ..original }),
            VersionRelation::LocalBehind
        );
        assert_eq!(
            compare_snapshots(original, SnapshotVersion { epoch: 2, ..original }),
            VersionRelation::Diverged
        );
        assert_eq!(
            compare_snapshots(original, SnapshotVersion { fingerprint: 12, ..original }),
            VersionRelation::Diverged
        );
    }
}