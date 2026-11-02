//! Bounded single-erasure parity for trusted immutable byte shards.
//! This codec does not authenticate recovered data or configure replication.

const MAX_STRIPE_SHARDS: usize = 16;
const MAX_SHARD_BYTES: usize = 8 * 1024 * 1024;

fn valid_stripe_count(count: usize) -> bool {
    (2..=MAX_STRIPE_SHARDS).contains(&count)
}

/// Compute zero-padded XOR parity directly into a caller-owned buffer.
///
/// Unequal source lengths are permitted. Source lengths must be no greater
/// than the destination length, which is the resulting parity shard size.
/// The caller is responsible for storing original lengths and checksums.
pub fn xor_parity_into(sources: &[&[u8]], destination: &mut [u8]) -> Result<(), &'static str> {
    if !valid_stripe_count(sources.len())
        || destination.len() > MAX_SHARD_BYTES
        || sources.iter().any(|source| source.len() > destination.len())
    {
        return Err("invalid XOR parity stripe layout");
    }
    destination.fill(0);
    for source in sources {
        for (output, input) in destination.iter_mut().zip(source.iter()) {
            *output ^= input;
        }
    }
    Ok(())
}

/// Encode parity with one bounded output allocation.
pub fn encode_xor_parity(sources: &[&[u8]]) -> Result<Vec<u8>, &'static str> {
    if !valid_stripe_count(sources.len()) {
        return Err("invalid XOR parity shard count");
    }
    let length = sources.iter().map(|source| source.len()).max().unwrap_or(0);
    if length > MAX_SHARD_BYTES {
        return Err("XOR parity shard exceeds size limit");
    }
    let mut parity = vec![0; length];
    xor_parity_into(sources, &mut parity)?;
    Ok(parity)
}

/// Reconstruct exactly one absent shard using stored parity.
///
/// The missing shard's original length must come from independently verified
/// stripe metadata. Recovered bytes require a separate strong integrity check
/// before publication; XOR alone cannot detect all corrupt data or parity.
pub fn recover_one_missing(
    shards: &[Option<&[u8]>],
    parity: &[u8],
    missing_length: usize,
) -> Result<Vec<u8>, &'static str> {
    if !valid_stripe_count(shards.len())
        || parity.len() > MAX_SHARD_BYTES
        || missing_length > parity.len()
    {
        return Err("invalid XOR recovery stripe layout");
    }
    let mut missing = None;
    for (index, shard) in shards.iter().enumerate() {
        if let Some(bytes) = shard {
            if bytes.len() > parity.len() {
                return Err("XOR source shard exceeds parity length");
            }
        } else if missing.replace(index).is_some() {
            return Err("XOR parity recovers exactly one missing shard");
        }
    }
    if missing.is_none() {
        return Err("XOR recovery requires one missing shard");
    }
    let mut recovered = parity.to_vec();
    for shard in shards.iter().flatten() {
        for (output, input) in recovered.iter_mut().zip(shard.iter()) {
            *output ^= input;
        }
    }
    // A shorter source was zero-padded during encoding. Any nonzero byte in
    // the missing shard's declared padding is inconsistent with the metadata.
    if recovered[missing_length..].iter().any(|&byte| byte != 0) {
        return Err("nonzero XOR parity padding");
    }
    recovered.truncate(missing_length);
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_every_missing_index_with_unequal_shard_lengths() {
        let shards: [&[u8]; 4] = [b"alpha", b"bravo-123", b"", b"delta"];
        let parity = encode_xor_parity(&shards).unwrap();
        assert_eq!(parity.len(), 9);
        for missing in 0..shards.len() {
            let available = shards
                .iter()
                .enumerate()
                .map(|(index, &shard)| (index != missing).then_some(shard))
                .collect::<Vec<_>>();
            let recovered = recover_one_missing(&available, &parity, shards[missing].len()).unwrap();
            assert_eq!(recovered, shards[missing]);
        }
    }

    #[test]
    fn rejects_multiple_failures_and_invalid_padding() {
        let shards: [&[u8]; 2] = [b"a", b"123"];
        let parity = encode_xor_parity(&shards).unwrap();
        assert!(recover_one_missing(&[None, None], &parity, 1).is_err());
        assert!(recover_one_missing(&[Some(b"a"), Some(b"123")], &parity, 0).is_err());
        assert!(recover_one_missing(&[None, Some(b"123")], &parity, 4).is_err());
        assert!(recover_one_missing(&[None, Some(b"1234")], &parity, 1).is_err());
        assert!(recover_one_missing(&[None, Some(b"123")], &parity, 0).is_err());
    }

    #[test]
    fn can_use_preallocated_buffer_and_zero_length_shards() {
        let shards: [&[u8]; 3] = [b"", b"", b""];
        assert_eq!(encode_xor_parity(&shards).unwrap(), Vec::<u8>::new());
        assert_eq!(recover_one_missing(&[None, Some(b""), Some(b"")], &[], 0).unwrap(), b"");
        let mut destination = [0xff; 4];
        xor_parity_into(&[b"test", b"test"], &mut destination).unwrap();
        assert_eq!(destination, [0; 4]);
    }

    #[test]
    fn enforces_hard_size_and_count_limits_before_writes() {
        let mut destination = [7u8; 3];
        assert!(xor_parity_into(&[b"abcd", b"x"], &mut destination).is_err());
        assert_eq!(destination, [7; 3]);
        assert!(encode_xor_parity(&[b"only-one"]).is_err());
        assert!(encode_xor_parity(&vec![b"x".as_slice(); 17]).is_err());
        let large = vec![0; MAX_SHARD_BYTES + 1];
        assert!(encode_xor_parity(&[&large, b""]).is_err());
    }
}


/// Two independent parity shards over GF(2^8), with primitive polynomial
/// x^8 + x^4 + x^3 + x^2 + 1. These are not integrity signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoubleParity {
    pub xor: Vec<u8>,
    pub weighted: Vec<u8>,
}

fn gf256_mul(mut a: u8, mut b: u8) -> u8 {
    let mut product = 0u8;
    for _ in 0..8 {
        if b & 1 != 0 {
            product ^= a;
        }
        let carry = a & 0x80 != 0;
        a <<= 1;
        if carry {
            a ^= 0x1d;
        }
        b >>= 1;
    }
    product
}

fn gf256_pow(mut base: u8, mut power: u16) -> u8 {
    let mut result = 1u8;
    while power != 0 {
        if power & 1 != 0 {
            result = gf256_mul(result, base);
        }
        base = gf256_mul(base, base);
        power >>= 1;
    }
    result
}

fn parity_coefficient(index: usize) -> u8 {
    // 2 is primitive under polynomial 0x11d; the first 16 coefficients
    // are all distinct and nonzero.
    gf256_pow(2, index as u16)
}

/// Encode XOR parity and a second independent Reed-Solomon-style parity.
/// Source lengths are externally retained; short shards are zero-padded.
pub fn encode_double_parity(sources: &[&[u8]]) -> Result<DoubleParity, &'static str> {
    if !valid_stripe_count(sources.len()) {
        return Err("invalid double parity shard count");
    }
    let width = sources.iter().map(|source| source.len()).max().unwrap_or(0);
    if width > MAX_SHARD_BYTES {
        return Err("double parity shard exceeds size limit");
    }
    let mut xor = vec![0u8; width];
    let mut weighted = vec![0u8; width];
    for (index, source) in sources.iter().enumerate() {
        let coefficient = parity_coefficient(index);
        for (offset, &byte) in source.iter().enumerate() {
            xor[offset] ^= byte;
            weighted[offset] ^= gf256_mul(coefficient, byte);
        }
    }
    Ok(DoubleParity { xor, weighted })
}

/// Recover one or two absent data shards from an intact pair of parity shards.
///
/// Original lengths must come from independently verified metadata. This
/// operation detects parity inconsistencies with one erasure, but cannot
/// authenticate data or detect arbitrary corruption with two erasures.
pub fn recover_double_erasure(
    shards: &[Option<&[u8]>],
    original_lengths: &[usize],
    parity: &DoubleParity,
) -> Result<Vec<(usize, Vec<u8>)>, &'static str> {
    if !valid_stripe_count(shards.len())
        || shards.len() != original_lengths.len()
        || parity.xor.len() != parity.weighted.len()
        || parity.xor.len() > MAX_SHARD_BYTES
    {
        return Err("invalid double parity recovery layout");
    }
    let width = parity.xor.len();
    let mut missing = Vec::with_capacity(2);
    for (index, (shard, &length)) in shards.iter().zip(original_lengths).enumerate() {
        if length > width {
            return Err("original shard length exceeds parity width");
        }
        match shard {
            Some(bytes) if bytes.len() != length => {
                return Err("available shard length does not match manifest");
            }
            None => missing.push(index),
            _ => {}
        }
    }
    if missing.is_empty() || missing.len() > 2 {
        return Err("double parity requires one or two missing shards");
    }

    let mut xor = parity.xor.clone();
    let mut weighted = parity.weighted.clone();
    for (index, shard) in shards.iter().enumerate() {
        if let Some(bytes) = shard {
            let coefficient = parity_coefficient(index);
            for (offset, &byte) in bytes.iter().enumerate() {
                xor[offset] ^= byte;
                weighted[offset] ^= gf256_mul(coefficient, byte);
            }
        }
    }

    let recovered = if missing.len() == 1 {
        let coefficient = parity_coefficient(missing[0]);
        if xor
            .iter()
            .zip(&weighted)
            .any(|(&byte, &check)| gf256_mul(coefficient, byte) != check)
        {
            return Err("double parity syndromes disagree");
        }
        vec![(missing[0], xor)]
    } else {
        let left = parity_coefficient(missing[0]);
        let right = parity_coefficient(missing[1]);
        let divisor = left ^ right;
        if divisor == 0 {
            return Err("duplicate parity coefficients");
        }
        let inverse = gf256_pow(divisor, 254);
        let mut a = Vec::with_capacity(width);
        let mut b = Vec::with_capacity(width);
        for (&p, &q) in xor.iter().zip(&weighted) {
            let first = gf256_mul(inverse, q ^ gf256_mul(right, p));
            a.push(first);
            b.push(p ^ first);
        }
        vec![(missing[0], a), (missing[1], b)]
    };

    let mut output = recovered;
    for (index, data) in &mut output {
        let length = original_lengths[*index];
        if data[length..].iter().any(|&byte| byte != 0) {
            return Err("nonzero recovered parity padding");
        }
        data.truncate(length);
    }
    Ok(output)
}

#[cfg(test)]
mod double_parity_tests {
    use super::*;

    #[test]
    fn all_one_and_two_erasure_combinations_roundtrip() {
        let originals: [&[u8]; 5] = [b"hello", b"abc", b"", b"longer-shard", b"xyz"];
        let parity = encode_double_parity(&originals).unwrap();
        let lengths = originals.iter().map(|part| part.len()).collect::<Vec<_>>();
        for left in 0..originals.len() {
            for right in left..originals.len() {
                let shards = originals
                    .iter()
                    .enumerate()
                    .map(|(index, &part)| {
                        (index != left && index != right).then_some(part)
                    })
                    .collect::<Vec<_>>();
                let recovery = recover_double_erasure(&shards, &lengths, &parity).unwrap();
                assert_eq!(recovery[0], (left, originals[left].to_vec()));
                if left != right {
                    assert_eq!(recovery[1], (right, originals[right].to_vec()));
                }
            }
        }
    }

    #[test]
    fn catches_bad_layouts_padding_and_single_syndrome_corruption() {
        let originals: [&[u8]; 3] = [b"aa", b"bbb", b"c"];
        let parity = encode_double_parity(&originals).unwrap();
        assert!(recover_double_erasure(
            &[None, None, None],
            &[2, 3, 1],
            &parity
        )
        .is_err());
        assert!(recover_double_erasure(
            &[None, Some(b"bb"), Some(b"c")],
            &[2, 3, 1],
            &parity
        )
        .is_err());
        let mut altered = parity.clone();
        altered.weighted[0] ^= 1;
        assert!(recover_double_erasure(
            &[None, Some(b"bbb"), Some(b"c")],
            &[2, 3, 1],
            &altered
        )
        .is_err());
        assert!(recover_double_erasure(
            &[None, Some(b"bbb"), Some(b"c")],
            &[1, 3, 1],
            &parity
        )
        .is_err());
        assert!(encode_double_parity(&[b"only-one"]).is_err());
    }
}
