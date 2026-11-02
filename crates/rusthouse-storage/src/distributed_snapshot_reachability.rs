//! Conservative, branch-aware garbage-collection candidate planning.
//! Never delete files based on this result without verifying all live pins.
use crate::distributed_ancestry::{SnapshotAncestry, SnapshotParent};
use crate::distributed_lineage::SnapshotVersion;
use std::collections::{HashMap, HashSet};

const MAX_PINNED_HEADS: usize = 16_384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachabilityPlan {
    pub keep: Vec<SnapshotVersion>,
    pub candidates: Vec<SnapshotVersion>,
}

/// Every active reader, replica head, and recovery checkpoint must be pinned.
/// This function only reasons about the supplied immutable ancestry forest.
pub fn plan_snapshot_reachability(
    records: &[SnapshotParent],
    pinned_heads: &[SnapshotVersion],
) -> Result<ReachabilityPlan, &'static str> {
    SnapshotAncestry::new(records)?;
    if pinned_heads.is_empty() || pinned_heads.len() > MAX_PINNED_HEADS {
        return Err("snapshot reachability needs bounded live heads");
    }
    let by_id: HashMap<(u64, u64), SnapshotParent> = records.iter()
        .map(|&record| ((record.snapshot.epoch, record.snapshot.generation), record))
        .collect();
    let mut reachable = HashSet::new();
    for &pin in pinned_heads {
        let mut current = pin;
        loop {
            let key = (current.epoch, current.generation);
            let record = by_id.get(&key).ok_or("pinned snapshot is unknown")?;
            if record.snapshot.fingerprint != current.fingerprint {
                return Err("pinned snapshot fingerprint mismatch");
            }
            if !reachable.insert(key) {
                break;
            }
            match record.parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
    }
    let mut keep = Vec::new();
    let mut candidates = Vec::new();
    for record in records {
        if reachable.contains(&(record.snapshot.epoch, record.snapshot.generation)) {
            keep.push(record.snapshot);
        } else {
            candidates.push(record.snapshot);
        }
    }
    keep.sort_unstable_by_key(|v| (v.epoch, v.generation));
    candidates.sort_unstable_by_key(|v| (v.epoch, v.generation));
    Ok(ReachabilityPlan { keep, candidates })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(generation: u64) -> SnapshotVersion {
        SnapshotVersion { epoch: 1, generation, fingerprint: generation as u128 }
    }

    #[test]
    fn pinned_branch_keeps_ancestors_but_not_unrelated_sibling() {
        let nodes = [
            SnapshotParent { snapshot: v(1), parent: None },
            SnapshotParent { snapshot: v(2), parent: Some(v(1)) },
            SnapshotParent { snapshot: v(3), parent: Some(v(1)) },
            SnapshotParent { snapshot: v(4), parent: Some(v(2)) },
        ];
        let plan = plan_snapshot_reachability(&nodes, &[v(4)]).unwrap();
        assert_eq!(plan.keep, [v(1), v(2), v(4)]);
        assert_eq!(plan.candidates, [v(3)]);
        let all = plan_snapshot_reachability(&nodes, &[v(4), v(3)]).unwrap();
        assert!(all.candidates.is_empty());
    }

    #[test]
    fn unknown_or_rewritten_pins_cannot_trigger_deletion() {
        let nodes = [SnapshotParent { snapshot: v(1), parent: None }];
        assert!(plan_snapshot_reachability(&nodes, &[]).is_err());
        assert!(plan_snapshot_reachability(&nodes, &[v(2)]).is_err());
        assert!(plan_snapshot_reachability(
            &nodes, &[SnapshotVersion { fingerprint: 99, ..v(1) }]
        ).is_err());
    }
}

#[cfg(test)]
mod duplicate_live_head_tests {
    use super::*;

    #[test]
    fn many_readers_may_pin_the_same_snapshot() {
        let version = SnapshotVersion { epoch: 1, generation: 1, fingerprint: 19 };
        let records = [SnapshotParent { snapshot: version, parent: None }];
        let duplicate_pins = vec![version; 128];
        let plan = plan_snapshot_reachability(&records, &duplicate_pins).unwrap();
        assert_eq!(plan.keep, [version]);
        assert!(plan.candidates.is_empty());
        assert!(plan_snapshot_reachability(
            &records,
            &vec![version; MAX_PINNED_HEADS + 1],
        ).is_err());
    }
}
