//! Three-way reconciliation of immutable object inventories.
//! A plan containing conflicts must never be published automatically.
use crate::distributed_copy_diff::{ObjectMetadata, plan_replica_copies};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreeWayManifestPlan {
    pub merged: Vec<ObjectMetadata>,
    pub conflicts: Vec<String>,
}


impl ThreeWayManifestPlan {
    /// Return a complete merged inventory only when every key was resolved.
    pub fn publishable_objects(&self) -> Result<&[ObjectMetadata], &'static str> {
        if !self.conflicts.is_empty() {
            return Err("manifest merge contains unresolved conflicts");
        }
        Ok(&self.merged)
    }
}

fn same(a: Option<&ObjectMetadata>, b: Option<&ObjectMetadata>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.bytes == b.bytes && a.crc32c == b.crc32c,
        _ => false,
    }
}

/// Compute a non-destructive merge from a shared immutable base.
/// CRC32C and size compare stored metadata, not cryptographic byte identity.
pub fn plan_three_way_manifest_merge(
    base: &[ObjectMetadata],
    local: &[ObjectMetadata],
    remote: &[ObjectMetadata],
) -> Result<ThreeWayManifestPlan, &'static str> {
    // Reuse the existing strict key canonicalization and duplicate checks.
    for inventory in [base, local, remote] {
        plan_replica_copies(inventory, &[])?;
    }
    fn index<'a>(records: &'a [ObjectMetadata]) -> BTreeMap<&'a str, &'a ObjectMetadata> {
        records
            .iter()
            .map(|record| (record.key.as_str(), record))
            .collect()
    }
    let original = index(base);
    let left = index(local);
    let right = index(remote);
    let keys = original.keys().chain(left.keys()).chain(right.keys())
        .copied().collect::<BTreeSet<_>>();
    let mut merged = Vec::new();
    let mut conflicts = Vec::new();
    for key in keys {
        let base = original.get(key).copied();
        let local = left.get(key).copied();
        let remote = right.get(key).copied();
        let chosen = if same(local, remote) {
            local
        } else if same(local, base) {
            remote
        } else if same(remote, base) {
            local
        } else {
            conflicts.push(key.to_owned());
            None
        };
        if let Some(record) = chosen { merged.push(record.clone()); }
    }
    Ok(ThreeWayManifestPlan { merged, conflicts })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn obj(key: &str, checksum: u32) -> ObjectMetadata {
        ObjectMetadata { key: key.into(), bytes: 8, crc32c: checksum }
    }
    #[test]
    fn merges_disjoint_changes_and_deletions() {
        let base = [obj("old", 1), obj("untouched", 2)];
        let local = [obj("untouched", 2), obj("local", 3)];
        let remote = [obj("old", 1), obj("untouched", 2), obj("remote", 4)];
        let plan = plan_three_way_manifest_merge(&base, &local, &remote).unwrap();
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.merged.iter().map(|x| x.key.as_str()).collect::<Vec<_>>(),
            ["local", "remote", "untouched"]);
    }
    #[test]
    fn concurrent_edits_and_delete_modify_are_conflicts() {
        let base = [obj("a", 1)];
        let plan = plan_three_way_manifest_merge(&base, &[obj("a", 2)], &[obj("a", 3)]).unwrap();
        assert_eq!(plan.conflicts, ["a"]);
        let plan = plan_three_way_manifest_merge(&base, &[], &[obj("a", 2)]).unwrap();
        assert_eq!(plan.conflicts, ["a"]);
    }
    #[test]
    fn duplicate_inventory_keys_fail_closed() {
        assert!(plan_three_way_manifest_merge(&[obj("a", 1), obj("a", 1)], &[], &[]).is_err());
    }
}

#[cfg(test)]
mod publishable_manifest_tests {
    use super::*;

    fn object(key: &str, crc32c: u32) -> ObjectMetadata {
        ObjectMetadata { key: key.into(), bytes: 10, crc32c }
    }

    #[test]
    fn unresolved_conflicts_cannot_produce_publishable_manifest() {
        let base = [object("a", 1)];
        let conflict = plan_three_way_manifest_merge(
            &base, &[object("a", 2)], &[object("a", 3)],
        ).unwrap();
        assert_eq!(conflict.conflicts, ["a"]);
        assert!(conflict.publishable_objects().is_err());
    }

    #[test]
    fn conflict_free_plan_can_be_published() {
        let plan = plan_three_way_manifest_merge(
            &[], &[object("local", 1)], &[object("remote", 2)],
        ).unwrap();
        assert_eq!(plan.publishable_objects().unwrap(), plan.merged.as_slice());
    }
}
