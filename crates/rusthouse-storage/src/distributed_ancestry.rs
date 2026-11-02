//! Validate immutable snapshot ancestry and query common ancestors in O(log N).
//! A parent reference is explicit metadata, not proof of a committed write.

use crate::distributed_lineage::SnapshotVersion;
use std::collections::HashMap;

const MAX_SNAPSHOTS: usize = 16_384;
const JUMP_LEVELS: usize = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotParent {
    pub snapshot: SnapshotVersion,
    pub parent: Option<SnapshotVersion>,
}

struct IndexedSnapshot {
    version: SnapshotVersion,
    depth: usize,
    jumps: [Option<usize>; JUMP_LEVELS],
}

/// Verified, bounded forest of strictly increasing generation histories.
pub struct SnapshotAncestry {
    nodes: Vec<IndexedSnapshot>,
    by_generation: HashMap<(u64, u64), usize>,
}

impl SnapshotAncestry {
    /// Reject missing parents, changed fingerprints, duplicate generations,
    /// invalid epochs, and ancestry that does not strictly increase.
    pub fn new(records: &[SnapshotParent]) -> Result<Self, &'static str> {
        if records.is_empty() || records.len() > MAX_SNAPSHOTS {
            return Err("invalid snapshot ancestry size");
        }
        let mut sorted = records.to_vec();
        sorted.sort_unstable_by_key(|record| (record.snapshot.epoch, record.snapshot.generation));
        let mut nodes: Vec<IndexedSnapshot> = Vec::with_capacity(sorted.len());
        let mut by_generation = HashMap::with_capacity(sorted.len());
        for record in sorted {
            let version = record.snapshot;
            if version.epoch == 0 || version.generation == 0 {
                return Err("invalid snapshot version");
            }
            let key = (version.epoch, version.generation);
            if by_generation.contains_key(&key) {
                return Err("duplicate snapshot generation");
            }
            let mut jumps = [None; JUMP_LEVELS];
            let mut depth = 0usize;
            if let Some(parent) = record.parent {
                if parent.epoch != version.epoch || parent.generation >= version.generation {
                    return Err("invalid snapshot parent order");
                }
                let parent_index = *by_generation
                    .get(&(parent.epoch, parent.generation))
                    .ok_or("missing snapshot ancestor")?;
                if nodes[parent_index].version.fingerprint != parent.fingerprint {
                    return Err("conflicting snapshot ancestor fingerprint");
                }
                jumps[0] = Some(parent_index);
                depth = nodes[parent_index].depth + 1;
                for level in 1..JUMP_LEVELS {
                    jumps[level] = jumps[level - 1]
                        .and_then(|ancestor| nodes[ancestor].jumps[level - 1]);
                }
            }
            by_generation.insert(key, nodes.len());
            nodes.push(IndexedSnapshot {
                version,
                depth,
                jumps,
            });
        }
        Ok(Self {
            nodes,
            by_generation,
        })
    }

    fn index(&self, version: SnapshotVersion) -> Result<usize, &'static str> {
        let index = *self
            .by_generation
            .get(&(version.epoch, version.generation))
            .ok_or("unknown snapshot generation")?;
        if self.nodes[index].version.fingerprint != version.fingerprint {
            return Err("snapshot fingerprint mismatch");
        }
        Ok(index)
    }

    /// Return the last known common parent, or None for disconnected histories.
    /// Differences in epoch never imply ancestry, even for equal generations.
    pub fn common_ancestor(
        &self,
        left: SnapshotVersion,
        right: SnapshotVersion,
    ) -> Result<Option<SnapshotVersion>, &'static str> {
        let mut a = self.index(left)?;
        let mut b = self.index(right)?;
        if left.epoch != right.epoch {
            return Ok(None);
        }
        if self.nodes[a].depth < self.nodes[b].depth {
            std::mem::swap(&mut a, &mut b);
        }
        let difference = self.nodes[a].depth - self.nodes[b].depth;
        for level in 0..JUMP_LEVELS {
            if difference & (1usize << level) != 0 {
                a = self.nodes[a].jumps[level].ok_or("invalid ancestry jump")?;
            }
        }
        if a == b {
            return Ok(Some(self.nodes[a].version));
        }
        for level in (0..JUMP_LEVELS).rev() {
            match (self.nodes[a].jumps[level], self.nodes[b].jumps[level]) {
                (Some(pa), Some(pb)) if pa != pb => {
                    a = pa;
                    b = pb;
                }
                _ => {}
            }
        }
        Ok(self.nodes[a].jumps[0].map(|index| self.nodes[index].version))
    }

    /// A head is an ancestor of itself, but different branches are not
    /// ordered merely because one has a higher generation number.
    pub fn is_ancestor(
        &self,
        ancestor: SnapshotVersion,
        head: SnapshotVersion,
    ) -> Result<bool, &'static str> {
        Ok(self.common_ancestor(ancestor, head)? == Some(ancestor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(epoch: u64, generation: u64, fingerprint: u128) -> SnapshotVersion {
        SnapshotVersion {
            epoch,
            generation,
            fingerprint,
        }
    }

    #[test]
    fn finds_branch_points_without_confusing_generation_order() {
        let root = v(1, 1, 11);
        let left = v(1, 2, 22);
        let right = v(1, 3, 33);
        let left_child = v(1, 4, 44);
        let index = SnapshotAncestry::new(&[
            SnapshotParent { snapshot: left_child, parent: Some(left) },
            SnapshotParent { snapshot: right, parent: Some(root) },
            SnapshotParent { snapshot: root, parent: None },
            SnapshotParent { snapshot: left, parent: Some(root) },
        ]).unwrap();
        assert_eq!(index.common_ancestor(left_child, right).unwrap(), Some(root));
        assert!(index.is_ancestor(root, right).unwrap());
        assert!(!index.is_ancestor(right, left_child).unwrap());
        assert_eq!(index.common_ancestor(left_child, left).unwrap(), Some(left));
    }

    #[test]
    fn rejects_conflicts_and_missing_ancestors() {
        let a = v(1, 1, 1);
        let b = v(1, 2, 2);
        assert!(SnapshotAncestry::new(&[
            SnapshotParent { snapshot: a, parent: None },
            SnapshotParent { snapshot: a, parent: None },
        ]).is_err());
        assert!(SnapshotAncestry::new(&[
            SnapshotParent { snapshot: b, parent: Some(a) },
        ]).is_err());
        assert!(SnapshotAncestry::new(&[
            SnapshotParent { snapshot: a, parent: None },
            SnapshotParent { snapshot: b, parent: Some(v(1, 1, 99)) },
        ]).is_err());
        assert!(SnapshotAncestry::new(&[
            SnapshotParent { snapshot: a, parent: Some(b) },
            SnapshotParent { snapshot: b, parent: Some(a) },
        ]).is_err());
    }

    #[test]
    fn epochs_and_disconnected_roots_never_alias() {
        let first = v(1, 1, 9);
        let second = v(2, 1, 9);
        let other = v(1, 3, 9);
        let index = SnapshotAncestry::new(&[
            SnapshotParent { snapshot: first, parent: None },
            SnapshotParent { snapshot: second, parent: None },
            SnapshotParent { snapshot: other, parent: None },
        ]).unwrap();
        assert_eq!(index.common_ancestor(first, second).unwrap(), None);
        assert_eq!(index.common_ancestor(first, other).unwrap(), None);
        assert!(index.common_ancestor(first, v(1, 1, 10)).is_err());
    }

    #[test]
    fn deep_history_uses_bounded_binary_lifting() {
        let history = (1..=MAX_SNAPSHOTS as u64)
            .map(|generation| SnapshotParent {
                snapshot: v(1, generation, u128::from(generation)),
                parent: (generation > 1)
                    .then(|| v(1, generation - 1, u128::from(generation - 1))),
            })
            .collect::<Vec<_>>();
        let index = SnapshotAncestry::new(&history).unwrap();
        assert_eq!(
            index.common_ancestor(v(1, 16_384, 16_384), v(1, 400, 400)).unwrap(),
            Some(v(1, 400, 400))
        );
        assert!(SnapshotAncestry::new(&history[..0]).is_err());
    }
}
