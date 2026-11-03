//! Cryptographic metadata absence proof using authenticated neighboring leaves.
//! The trusted Merkle root must come from an authenticated control plane.
//! A proof of missing metadata is not proof of absent physical file bytes.

use crate::distributed_copy_diff::ObjectMetadata;
use crate::distributed_merkle::{MerkleInventory, MerkleProof};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenNeighbor {
    pub object: ObjectMetadata,
    pub proof: MerkleProof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleAbsenceProof {
    pub leaf_count: usize,
    pub before: Option<ProvenNeighbor>,
    pub after: Option<ProvenNeighbor>,
}

/// Produce authenticated immediate neighbors of an absent key.
///
/// An existing key cannot be proven absent. Empty inventories have a zero-leaf
/// proof which must be checked against the authenticated empty-tree root.
pub fn prove_absence(
    inventory: &MerkleInventory,
    key: &str,
) -> Result<MerkleAbsenceProof, &'static str> {
    let objects = inventory.objects();
    if key.is_empty() {
        return Err("absence proof requires a key");
    }
    if objects.is_empty() {
        return Ok(MerkleAbsenceProof {
            leaf_count: 0,
            before: None,
            after: None,
        });
    }
    let insertion = objects.partition_point(|entry| entry.key.as_str() < key);
    if objects.get(insertion).is_some_and(|entry| entry.key == key) {
        return Err("key is present in inventory");
    }
    let neighbor = |index: usize| -> ProvenNeighbor {
        ProvenNeighbor {
            object: objects[index].clone(),
            proof: inventory.proof(index).expect("validated inventory index"),
        }
    };
    Ok(MerkleAbsenceProof {
        leaf_count: objects.len(),
        before: insertion.checked_sub(1).map(neighbor),
        after: (insertion < objects.len()).then(|| neighbor(insertion)),
    })
}

/// Verify that a missing key lies exactly between two authenticated,
/// adjacent leaves, or before the first or after the last authenticated leaf.
/// Forged indexes, bounds and proofs are rejected even if key ordering holds.
pub fn verify_absence(
    trusted_root: [u8; 32],
    key: &str,
    proof: &MerkleAbsenceProof,
) -> bool {
    if key.is_empty() {
        return false;
    }
    if proof.leaf_count == 0 {
        return proof.before.is_none()
            && proof.after.is_none()
            && MerkleInventory::new(&[])
                .is_ok_and(|inventory| inventory.root() == trusted_root);
    }
    let verified = |neighbor: &ProvenNeighbor| {
        neighbor.proof.leaf_count == proof.leaf_count
            && MerkleInventory::verify_proof(
                trusted_root,
                &neighbor.object,
                &neighbor.proof,
            )
    };
    match (&proof.before, &proof.after) {
        (Some(before), Some(after)) => {
            verified(before)
                && verified(after)
                && before.object.key.as_str() < key
                && key < after.object.key.as_str()
                && before.proof.leaf_index.checked_add(1)
                    == Some(after.proof.leaf_index)
        }
        (None, Some(after)) => {
            verified(after)
                && after.proof.leaf_index == 0
                && key < after.object.key.as_str()
        }
        (Some(before), None) => {
            verified(before)
                && before.proof.leaf_index.checked_add(1)
                    == Some(proof.leaf_count)
                && before.object.key.as_str() < key
        }
        (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str) -> ObjectMetadata {
        ObjectMetadata {
            key: key.to_owned(),
            bytes: key.len() as u64,
            crc32c: key.bytes().map(u32::from).sum(),
        }
    }

    #[test]
    fn proves_absence_before_between_and_after_known_leaves() {
        for keys in [
            vec!["m"],
            vec!["a", "m"],
            vec!["a", "c", "f", "m", "z"],
        ] {
            let objects = keys.iter().map(|key| entry(key)).collect::<Vec<_>>();
            let inventory = MerkleInventory::new(&objects).unwrap();
            for candidate in ["0", "b", "d", "k", "n", "zz"] {
                if keys.contains(&candidate) {
                    continue;
                }
                let proof = prove_absence(&inventory, candidate).unwrap();
                assert!(verify_absence(inventory.root(), candidate, &proof));
                assert!(!verify_absence(inventory.root(), "m", &proof));
                assert!(!verify_absence([0; 32], candidate, &proof));
            }
            for key in keys {
                assert!(prove_absence(&inventory, key).is_err());
            }
        }
    }

    #[test]
    fn rejects_forged_neighbors_and_missing_boundary_proofs() {
        let inventory = MerkleInventory::new(
            &[entry("a"), entry("c"), entry("e"), entry("g")],
        ).unwrap();
        let mut proof = prove_absence(&inventory, "d").unwrap();
        assert!(verify_absence(inventory.root(), "d", &proof));

        let mut changed = proof.clone();
        changed.before.as_mut().unwrap().object.crc32c ^= 1;
        assert!(!verify_absence(inventory.root(), "d", &changed));

        let mut changed = proof.clone();
        changed.after.as_mut().unwrap().proof.leaf_index = 3;
        assert!(!verify_absence(inventory.root(), "d", &changed));

        proof.after = None;
        assert!(!verify_absence(inventory.root(), "d", &proof));
        let first = prove_absence(&inventory, "0").unwrap();
        assert!(verify_absence(inventory.root(), "0", &first));
        let mut bad_first = first;
        bad_first.after.as_mut().unwrap().proof.leaf_index = 1;
        assert!(!verify_absence(inventory.root(), "0", &bad_first));
    }
}

#[cfg(test)]
mod empty_inventory_absence_tests {
    use super::*;

    #[test]
    fn only_the_authenticated_empty_root_proves_empty_inventory_absence() {
        let inventory = MerkleInventory::new(&[]).unwrap();
        let proof = prove_absence(&inventory, "missing").unwrap();
        assert_eq!(proof.leaf_count, 0);
        assert!(proof.before.is_none());
        assert!(proof.after.is_none());
        assert!(verify_absence(inventory.root(), "missing", &proof));
        assert!(!verify_absence([0; 32], "missing", &proof));
        assert!(!verify_absence(inventory.root(), "", &proof));

        let mut forged = proof.clone();
        forged.leaf_count = 1;
        assert!(!verify_absence(inventory.root(), "missing", &forged));
        assert!(prove_absence(&inventory, "").is_err());
    }

    #[test]
    fn nonempty_inventory_root_cannot_validate_empty_proof() {
        let empty = MerkleInventory::new(&[]).unwrap();
        let nonempty = MerkleInventory::new(&[ObjectMetadata {
            key: "part/data.bin".into(), bytes: 1, crc32c: 1,
        }]).unwrap();
        let proof = prove_absence(&empty, "missing").unwrap();
        assert!(!verify_absence(nonempty.root(), "missing", &proof));
    }
}
