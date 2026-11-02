//! SHA-256 Merkle fingerprints over validated replica inventory metadata.
//! Source CRC32C values are not cryptographic file digests. A matching root
//! proves metadata equality to the usual hash assumptions, not file integrity.

use crate::distributed_copy_diff::{ObjectMetadata, plan_replica_copies};

type Digest = [u8; 32];

const ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
    0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

// Self-contained SHA-256 for canonical bounded manifest nodes, without
// changing the workspace's dependency graph or lockfile.
fn sha256(bytes: &[u8]) -> Digest {
    let mut padded = Vec::with_capacity(bytes.len() + 72);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&((bytes.len() as u64) * 8).to_be_bytes());
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    for block in padded.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes(chunk.try_into().expect("four-byte word"));
        }
        for index in 16..64 {
            let a = words[index - 15];
            let b = words[index - 2];
            let sigma0 = a.rotate_right(7) ^ a.rotate_right(18) ^ (a >> 3);
            let sigma1 = b.rotate_right(17) ^ b.rotate_right(19) ^ (b >> 10);
            words[index] = words[index - 16]
                .wrapping_add(sigma0)
                .wrapping_add(words[index - 7])
                .wrapping_add(sigma1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let sigma1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(sigma1)
                .wrapping_add(choice)
                .wrapping_add(ROUND_CONSTANTS[index])
                .wrapping_add(words[index]);
            let sigma0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = sigma0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in state
            .iter_mut()
            .zip([a, b, c, d, e, f, g, h])
        {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut output = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}

fn leaf_digest(object: &ObjectMetadata) -> Digest {
    let mut bytes = Vec::with_capacity(1 + 2 + object.key.len() + 8 + 4);
    bytes.push(0);
    bytes.extend_from_slice(&(object.key.len() as u16).to_le_bytes());
    bytes.extend_from_slice(object.key.as_bytes());
    bytes.extend_from_slice(&object.bytes.to_le_bytes());
    bytes.extend_from_slice(&object.crc32c.to_le_bytes());
    sha256(&bytes)
}

fn parent_digest(left: Digest, right: Digest) -> Digest {
    let mut bytes = [0u8; 65];
    bytes[0] = 1;
    bytes[1..33].copy_from_slice(&left);
    bytes[33..].copy_from_slice(&right);
    sha256(&bytes)
}

/// Hash tree over canonical, lexicographically sorted object metadata.
pub struct MerkleInventory {
    objects: Vec<ObjectMetadata>,
    levels: Vec<Vec<Digest>>,
}

impl MerkleInventory {
    pub fn new(entries: &[ObjectMetadata]) -> Result<Self, &'static str> {
        // Reuse the existing canonical key grammar and duplicate detection.
        let objects = plan_replica_copies(entries, &[])?;
        let mut levels = Vec::new();
        let mut current = objects.iter().map(leaf_digest).collect::<Vec<_>>();
        if current.is_empty() {
            current.push(sha256(&[2]));
        }
        levels.push(current);
        let empty_right = sha256(&[3]);
        while levels.last().expect("at least one Merkle level").len() > 1 {
            let previous = levels.last().expect("at least one Merkle level");
            let next = previous
                .chunks(2)
                .map(|pair| parent_digest(pair[0], *pair.get(1).unwrap_or(&empty_right)))
                .collect::<Vec<_>>();
            levels.push(next);
        }
        Ok(Self { objects, levels })
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn root(&self) -> Digest {
        self.levels.last().expect("Merkle root exists")[0]
    }

    pub fn objects(&self) -> &[ObjectMetadata] {
        &self.objects
    }

    fn visit_changed(&self, other: &Self, level: usize, index: usize, out: &mut Vec<usize>) {
        if self.levels[level][index] == other.levels[level][index] {
            return;
        }
        if level == 0 {
            out.push(index);
        } else {
            for child in [2 * index, 2 * index + 1] {
                if child < self.levels[level - 1].len() {
                    self.visit_changed(other, level - 1, child, out);
                }
            }
        }
    }

    /// Return candidate changed leaf positions in sorted manifest order.
    ///
    /// Different inventory lengths return None because leaf indexes do not
    /// align. Consumers must match and verify underlying objects before
    /// repair; these metadata fingerprints are not file-byte attestation.
    pub fn differing_positions(&self, other: &Self) -> Option<Vec<usize>> {
        if self.len() != other.len() {
            return None;
        }
        let mut positions = Vec::new();
        let last = self.levels.len() - 1;
        self.visit_changed(other, last, 0, &mut positions);
        Some(positions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, bytes: u64, crc32c: u32) -> ObjectMetadata {
        ObjectMetadata { key: key.into(), bytes, crc32c }
    }

    #[test]
    fn sha256_matches_independent_nist_vectors() {
        let hex = |digest: Digest| digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert_eq!(
            hex(sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn order_independent_root_and_sparse_changed_leaves() {
        let objects = [
            entry("parts/a.bin", 4, 123),
            entry("parts/b.bin", 8, 456),
            entry("parts/c.bin", 12, 789),
        ];
        let a = MerkleInventory::new(&objects).unwrap();
        let mut shuffled = objects.to_vec();
        shuffled.reverse();
        let b = MerkleInventory::new(&shuffled).unwrap();
        assert_eq!(a.root(), b.root());
        assert_eq!(a.differing_positions(&b).unwrap(), Vec::<usize>::new());
        shuffled[1].crc32c += 1;
        let c = MerkleInventory::new(&shuffled).unwrap();
        assert_ne!(a.root(), c.root());
        assert_eq!(a.differing_positions(&c).unwrap(), vec![1]);
    }

    #[test]
    fn inventory_shape_changes_require_reconciliation() {
        let empty = MerkleInventory::new(&[]).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.differing_positions(&empty), Some(Vec::new()));
        let one = MerkleInventory::new(&[entry("a", 1, 1)]).unwrap();
        assert_eq!(one.differing_positions(&empty), None);
        assert!(MerkleInventory::new(&[entry("../escape", 1, 1)]).is_err());
        assert!(MerkleInventory::new(&[entry("a", 1, 1), entry("a", 1, 2)]).is_err());
    }

    #[test]
    fn one_changed_leaf_in_large_inventory_remains_localized() {
        let objects = (0..1025)
            .map(|index| entry(&format!("parts/{index:05}.bin"), index, index as u32))
            .collect::<Vec<_>>();
        let first = MerkleInventory::new(&objects).unwrap();
        let mut changed = objects;
        changed[513].bytes += 1;
        let second = MerkleInventory::new(&changed).unwrap();
        assert_eq!(first.differing_positions(&second), Some(vec![513]));
    }
}
