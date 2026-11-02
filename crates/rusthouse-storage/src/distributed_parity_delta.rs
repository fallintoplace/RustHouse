//! Recalculate dual erasure parity after replacing one immutable data shard.
//! This is an offline stripe transformation: the caller must publish an
//! entirely new parity generation, never overwrite an active immutable one.

use crate::distributed_parity::DoubleParity;

const MAX_SOURCE_SHARDS: usize = 16;
const MAX_STRIPE_BYTES: usize = 8 * 1024 * 1024;

fn gf_mul(mut left: u8, mut right: u8) -> u8 {
    let mut value = 0u8;
    for _ in 0..8 {
        if right & 1 != 0 {
            value ^= left;
        }
        let carry = left & 0x80 != 0;
        left <<= 1;
        if carry {
            left ^= 0x1d;
        }
        right >>= 1;
    }
    value
}

/// Update the two parity equations in O(max(old.len(), new.len())) time.
///
/// The result has the same stripe width as its input, so a replacement that
/// would increase the maximum stripe width must instead re-encode the stripe.
/// This operation does not authenticate input data or verify a parity digest.
/// Publish only after verifying the entire new generation's strong checksums.
pub fn replace_double_parity_source(
    parity: &mut DoubleParity,
    source_count: usize,
    changed_index: usize,
    old: &[u8],
    new: &[u8],
) -> Result<(), &'static str> {
    let width = parity.xor.len();
    if !(2..=MAX_SOURCE_SHARDS).contains(&source_count)
        || changed_index >= source_count
        || parity.weighted.len() != width
        || width > MAX_STRIPE_BYTES
        || old.len() > width
        || new.len() > width
    {
        return Err("invalid double parity replacement layout");
    }
    let mut factor = 1u8;
    for _ in 0..changed_index {
        factor = gf_mul(factor, 2);
    }
    let length = old.len().max(new.len());
    if length < 1024 {
        for offset in 0..length {
            let before = old.get(offset).copied().unwrap_or(0);
            let after = new.get(offset).copied().unwrap_or(0);
            let delta = before ^ after;
            parity.xor[offset] ^= delta;
            parity.weighted[offset] ^= gf_mul(factor, delta);
        }
    } else {
        // The changed source has one coefficient for all of its bytes.
        let products: [u8; 256] = std::array::from_fn(|byte| gf_mul(factor, byte as u8));
        for offset in 0..length {
            let before = old.get(offset).copied().unwrap_or(0);
            let after = new.get(offset).copied().unwrap_or(0);
            let delta = before ^ after;
            parity.xor[offset] ^= delta;
            parity.weighted[offset] ^= products[usize::from(delta)];
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed_parity::{encode_double_parity, recover_double_erasure};

    #[test]
    fn replaces_every_source_and_restores_all_erasure_combinations() {
        let original: Vec<Vec<u8>> = vec![
            b"hello".to_vec(),
            b"lots-of-bytes".to_vec(),
            vec![],
            b"abc".to_vec(),
            b"xyz".to_vec(),
        ];
        let originals = original.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let initial = encode_double_parity(&originals).unwrap();
        for changed in 0..original.len() {
            for replacement in [
                b"".as_slice(),
                b"x".as_slice(),
                b"changed".as_slice(),
                b"another test".as_slice(),
            ] {
                let mut updated = original.clone();
                updated[changed] = replacement.to_vec();
                let mut incremental = initial.clone();
                replace_double_parity_source(
                    &mut incremental,
                    original.len(),
                    changed,
                    &original[changed],
                    replacement,
                )
                .unwrap();
                let slices = updated.iter().map(Vec::as_slice).collect::<Vec<_>>();
                // Incremental replacement intentionally keeps the original stripe width.
                // A full re-encode may shrink when the longest shard is replaced.
                let mut expected = encode_double_parity(&slices).unwrap();
                expected.xor.resize(initial.xor.len(), 0);
                expected.weighted.resize(initial.weighted.len(), 0);
                assert_eq!(incremental, expected);
                let lengths = updated.iter().map(Vec::len).collect::<Vec<_>>();
                for left in 0..updated.len() {
                    for right in left..updated.len() {
                        let damaged = slices
                            .iter()
                            .enumerate()
                            .map(|(index, &bytes)| {
                                (index != left && index != right).then_some(bytes)
                            })
                            .collect::<Vec<_>>();
                        let recovered =
                            recover_double_erasure(&damaged, &lengths, &incremental).unwrap();
                        assert_eq!(recovered[0], (left, updated[left].clone()));
                        if left != right {
                            assert_eq!(recovered[1], (right, updated[right].clone()));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn long_updates_match_reencoding_across_all_byte_values() {
        let old = (0..4096).map(|index| index as u8).collect::<Vec<_>>();
        let other = (0..4096).map(|index| (index * 53) as u8).collect::<Vec<_>>();
        let replacement = (0..3073).map(|index| (index * 29 + 17) as u8).collect::<Vec<_>>();
        let initial = encode_double_parity(&[&old, &other]).unwrap();
        let mut updated = initial.clone();
        replace_double_parity_source(&mut updated, 2, 0, &old, &replacement).unwrap();
        let mut expected = encode_double_parity(&[&replacement, &other]).unwrap();
        expected.xor.resize(initial.xor.len(), 0);
        expected.weighted.resize(initial.weighted.len(), 0);
        assert_eq!(updated, expected);
    }

    #[test]
    fn rejects_invalid_updates_without_modifying_destination() {
        let source: [&[u8]; 2] = [b"abc", b"xyz"];
        let original = encode_double_parity(&source).unwrap();
        for (count, index, old, new) in [
            (1, 0, b"abc".as_slice(), b"x".as_slice()),
            (2, 2, b"abc".as_slice(), b"x".as_slice()),
            (2, 0, b"abc".as_slice(), b"longer".as_slice()),
        ] {
            let mut parity = original.clone();
            assert!(replace_double_parity_source(&mut parity, count, index, old, new).is_err());
            assert_eq!(parity, original);
        }
        let mut malformed = original.clone();
        malformed.weighted.pop();
        assert!(replace_double_parity_source(&mut malformed, 2, 0, b"abc", b"x").is_err());
    }
}
