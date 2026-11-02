//! Monotonic client snapshot floors with explicit ancestry validation.
//! Caller must still verify that a read actually uses the required snapshot.
use crate::distributed_ancestry::{SnapshotAncestry, SnapshotParent};
use crate::distributed_lineage::SnapshotVersion;

#[derive(Clone, Debug)]
pub struct SnapshotSessionFloor {
    floor: SnapshotVersion,
}

impl SnapshotSessionFloor {
    pub fn new(
        records: &[SnapshotParent],
        initial: SnapshotVersion,
    ) -> Result<Self, &'static str> {
        let ancestry = SnapshotAncestry::new(records)?;
        if !ancestry.is_ancestor(initial, initial)? {
            return Err("invalid initial session snapshot");
        }
        Ok(Self { floor: initial })
    }

    pub fn required_snapshot(&self) -> SnapshotVersion { self.floor }

    /// A newer generation is not enough: it must descend from the last
    /// accepted snapshot in the same explicitly verified ancestry forest.
    pub fn advance(
        &mut self,
        ancestry: &SnapshotAncestry,
        next: SnapshotVersion,
    ) -> Result<(), &'static str> {
        if !ancestry.is_ancestor(self.floor, next)? {
            return Err("snapshot would violate monotonic session ancestry");
        }
        self.floor = next;
        Ok(())
    }

    /// A read at this snapshot is admissible only if it contains all
    /// generations covered by the required floor.
    pub fn permits(
        &self,
        ancestry: &SnapshotAncestry,
        candidate: SnapshotVersion,
    ) -> Result<bool, &'static str> {
        ancestry.is_ancestor(self.floor, candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(generation: u64) -> SnapshotVersion {
        SnapshotVersion { epoch: 1, generation, fingerprint: generation as u128 }
    }
    fn ancestry() -> Vec<SnapshotParent> {
        vec![
            SnapshotParent { snapshot: v(1), parent: None },
            SnapshotParent { snapshot: v(2), parent: Some(v(1)) },
            SnapshotParent { snapshot: v(3), parent: Some(v(1)) },
            SnapshotParent { snapshot: v(4), parent: Some(v(2)) },
        ]
    }

    #[test]
    fn rejects_backward_reads_and_divergent_higher_generations() {
        let records = ancestry();
        let ancestry = SnapshotAncestry::new(&records).unwrap();
        let mut session = SnapshotSessionFloor::new(&records, v(2)).unwrap();
        assert!(session.permits(&ancestry, v(4)).unwrap());
        assert!(!session.permits(&ancestry, v(3)).unwrap());
        session.advance(&ancestry, v(4)).unwrap();
        assert!(!session.permits(&ancestry, v(2)).unwrap());
        assert!(session.advance(&ancestry, v(3)).is_err());
        assert_eq!(session.required_snapshot(), v(4));
    }

    #[test]
    fn unknown_versions_do_not_advance_session() {
        let records = ancestry();
        let ancestry = SnapshotAncestry::new(&records).unwrap();
        let mut session = SnapshotSessionFloor::new(&records, v(1)).unwrap();
        let unknown = SnapshotVersion { fingerprint: 0, ..v(4) };
        assert!(session.advance(&ancestry, unknown).is_err());
        assert_eq!(session.required_snapshot(), v(1));
    }
}
