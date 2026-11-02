//! Locate one unknown corrupted data shard using dual GF256 parity syndromes.
//! This is an offline diagnostic recovery aid. A successful syndrome match is
//! not an integrity signature; independently verify repaired object digests.

use crate::distributed_parity::{DoubleParity, encode_double_parity};

fn gf256_mul(mut left: u8, mut right: u8) -> u8 {
    let mut result = 0u8;
    for _ in 0..8 {
        if right & 1 != 0 {
            result ^= left;
        }
        let high = left & 0x80 != 0;
        left <<= 1;
        if high {
            left ^= 0x1d;
        }
        right >>= 1;
    }
    result
}

/// Diagnose and correct exactly one unknown data corruption using P/Q parity.
///
/// Returns None if the supplied data reproduces both parity shards. If P/Q
/// syndromes identify one candidate index, return its corrected bytes.
/// There is no guarantee that corrupt parity or multiple data errors cannot
/// masquerade as a single error. Verify every repaired byte with an
/// independent authenticated digest before publishing anything.
pub fn correct_one_unknown_corruption(
    shards: &[&[u8]],
    parity: &DoubleParity,
) -> Result<Option<(usize, Vec<u8>)>, &'static str> {
    let width = parity.xor.len();
    if !(2..=16).contains(&shards.len())
        || width != parity.weighted.len()
        || width > 8 * 1024 * 1024
        || shards.iter().map(|part| part.len()).max() != Some(width)
    {
        return Err("invalid unknown-error correction stripe");
    }
    let current = encode_double_parity(shards)?;
    if current == *parity {
        return Ok(None);
    }
    // Any nonzero XOR syndrome fixes the candidate coefficient. Avoid
    // rescanning an entire stripe for every replica that cannot match it.
    let pivot = current
        .xor
        .iter()
        .zip(&parity.xor)
        .position(|(&actual, &expected)| actual != expected)
        .ok_or("uncorrectable dual-parity syndrome")?;
    let p = current.xor[pivot] ^ parity.xor[pivot];
    let q = current.weighted[pivot] ^ parity.weighted[pivot];
    let mut coefficient = 1u8;
    let mut candidate = None;
    for (index, shard) in shards.iter().enumerate() {
        if pivot < shard.len() && gf256_mul(coefficient, p) == q {
            candidate = Some((index, coefficient));
            break;
        }
        coefficient = gf256_mul(coefficient, 2);
    }
    let (index, coefficient) = candidate.ok_or("uncorrectable dual-parity syndrome")?;
    if width < 1024 {
        for offset in 0..width {
            let p = current.xor[offset] ^ parity.xor[offset];
            let q = current.weighted[offset] ^ parity.weighted[offset];
            if gf256_mul(coefficient, p) != q || (offset >= shards[index].len() && p != 0) {
                return Err("uncorrectable dual-parity syndrome");
            }
        }
    } else {
        let products: [u8; 256] = std::array::from_fn(|byte| gf256_mul(coefficient, byte as u8));
        for offset in 0..width {
            let p = current.xor[offset] ^ parity.xor[offset];
            let q = current.weighted[offset] ^ parity.weighted[offset];
            if products[usize::from(p)] != q || (offset >= shards[index].len() && p != 0) {
                return Err("uncorrectable dual-parity syndrome");
            }
        }
    }
    let restored = shards[index]
        .iter()
        .zip(current.xor.iter().zip(&parity.xor))
        .map(|(&byte, (&actual, &expected))| byte ^ actual ^ expected)
        .collect::<Vec<_>>();
    Ok(Some((index, restored)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_every_single_shard_and_byte_corruption() {
        let originals = [
            b"alpha".as_slice(),
            b"charlie".as_slice(),
            b"".as_slice(),
            b"much-longer".as_slice(),
            b"omega".as_slice(),
        ];
        let parity = encode_double_parity(&originals).unwrap();
        assert_eq!(correct_one_unknown_corruption(&originals, &parity).unwrap(), None);
        for index in 0..originals.len() {
            if originals[index].is_empty() {
                continue;
            }
            for offset in 0..originals[index].len() {
                let mut damaged = originals.iter().map(|s| s.to_vec()).collect::<Vec<_>>();
                damaged[index][offset] ^= 0x97;
                let references = damaged.iter().map(Vec::as_slice).collect::<Vec<_>>();
                assert_eq!(
                    correct_one_unknown_corruption(&references, &parity).unwrap(),
                    Some((index, originals[index].to_vec())),
                    "index={index} offset={offset}"
                );
            }
        }
    }

    #[test]
    fn locates_late_corruption_without_rechecking_every_replica() {
        let originals = (0..8)
            .map(|shard| {
                (0..8192)
                    .map(|offset| (offset * 31 + shard * 17) as u8)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let source = originals.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let parity = encode_double_parity(&source).unwrap();
        let mut damaged = originals.clone();
        damaged[7][8191] ^= 0x9d;
        let corrupted = damaged.iter().map(Vec::as_slice).collect::<Vec<_>>();
        assert_eq!(
            correct_one_unknown_corruption(&corrupted, &parity).unwrap(),
            Some((7, originals[7].clone()))
        );
        damaged[2][4096] ^= 0x7e;
        let twice = damaged.iter().map(Vec::as_slice).collect::<Vec<_>>();
        assert!(correct_one_unknown_corruption(&twice, &parity).is_err());
    }

    #[test]
    fn rejects_multiple_corruptions_and_bad_stripe_shapes() {
        let originals = [b"hello".as_slice(), b"world".as_slice(), b"ok".as_slice()];
        let parity = encode_double_parity(&originals).unwrap();
        let damaged = [b"hallo".as_slice(), b"werld".as_slice(), b"ok".as_slice()];
        assert!(correct_one_unknown_corruption(&damaged, &parity).is_err());
        assert!(correct_one_unknown_corruption(&originals[..1], &parity).is_err());
        let mut bad = parity.clone();
        bad.weighted.pop();
        assert!(correct_one_unknown_corruption(&originals, &bad).is_err());
        let wrong_width = [b"hello".as_slice(), b"world".as_slice(), b"abcdef".as_slice()];
        assert!(correct_one_unknown_corruption(&wrong_width, &parity).is_err());
    }
}
