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

/// Verify a complete XOR parity stripe without allocating a second shard.
///
/// This detects mismatches against the supplied bytes, but it is not a
/// cryptographic integrity check. Authenticate immutable objects separately.
pub fn verify_xor_parity(sources: &[&[u8]], parity: &[u8]) -> Result<bool, &'static str> {
    if !valid_stripe_count(sources.len())
        || parity.len() > MAX_SHARD_BYTES
        || sources.iter().map(|source| source.len()).max() != Some(parity.len())
    {
        return Err("invalid XOR parity stripe layout");
    }
    for (index, &expected) in parity.iter().enumerate() {
        let mut actual = 0u8;
        for source in sources {
            actual ^= source.get(index).copied().unwrap_or(0);
        }
        if actual != expected {
            return Ok(false);
        }
    }
    Ok(true)
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
    fn complete_parity_verification_catches_corruption_without_recovery() {
        let sources: [&[u8]; 3] = [b"a", b"bcd", b"e"];
        let parity = encode_xor_parity(&sources).unwrap();
        assert!(verify_xor_parity(&sources, &parity).unwrap());

        let mut damaged = parity.clone();
        damaged[1] ^= 0x80;
        assert!(!verify_xor_parity(&sources, &damaged).unwrap());
        assert!(verify_xor_parity(&sources, &parity[..2]).is_err());
        assert!(verify_xor_parity(&sources[..1], &parity).is_err());

        let empty: [&[u8]; 2] = [b"", b""];
        assert!(verify_xor_parity(&empty, &[]).unwrap());
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


/// Three parity equations for recovery of up to three erased data shards.
/// A parity checksum alone is not a proof of authentic object contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TripleParity {
    pub xor: Vec<u8>,
    pub weighted: Vec<u8>,
    pub squared: Vec<u8>,
}

/// Encode P, Q and R parity with independent GF(256) coefficients.
pub fn encode_triple_parity(sources: &[&[u8]]) -> Result<TripleParity, &'static str> {
    if !valid_stripe_count(sources.len()) {
        return Err("invalid triple parity shard count");
    }
    let width = sources.iter().map(|part| part.len()).max().unwrap_or(0);
    if width > MAX_SHARD_BYTES {
        return Err("triple parity shard exceeds size limit");
    }
    let mut parity = TripleParity {
        xor: vec![0; width],
        weighted: vec![0; width],
        squared: vec![0; width],
    };
    for (index, source) in sources.iter().enumerate() {
        let coefficient = parity_coefficient(index);
        let squared = gf256_mul(coefficient, coefficient);
        for (offset, &byte) in source.iter().enumerate() {
            parity.xor[offset] ^= byte;
            parity.weighted[offset] ^= gf256_mul(coefficient, byte);
            parity.squared[offset] ^= gf256_mul(squared, byte);
        }
    }
    Ok(parity)
}

/// Recover one, two or three absent data shards using bounded Gaussian
/// elimination. Original lengths must be obtained from trusted metadata.
/// With all three parity equations consumed, there is no residual syndrome
/// to distinguish corrupt surviving data from correct data.
pub fn recover_triple_erasure(
    shards: &[Option<&[u8]>],
    original_lengths: &[usize],
    parity: &TripleParity,
) -> Result<Vec<(usize, Vec<u8>)>, &'static str> {
    if !valid_stripe_count(shards.len())
        || shards.len() != original_lengths.len()
        || parity.xor.len() != parity.weighted.len()
        || parity.xor.len() != parity.squared.len()
        || parity.xor.len() > MAX_SHARD_BYTES
    {
        return Err("invalid triple parity stripe layout");
    }
    let width = parity.xor.len();
    let mut missing = Vec::with_capacity(3);
    let mut residual = [
        parity.xor.clone(),
        parity.weighted.clone(),
        parity.squared.clone(),
    ];
    for (index, (shard, &length)) in shards.iter().zip(original_lengths).enumerate() {
        if length > width {
            return Err("original shard length exceeds parity width");
        }
        match shard {
            Some(data) => {
                if data.len() != length {
                    return Err("available shard length does not match manifest");
                }
                let coefficient = parity_coefficient(index);
                let squared = gf256_mul(coefficient, coefficient);
                for (offset, &byte) in data.iter().enumerate() {
                    residual[0][offset] ^= byte;
                    residual[1][offset] ^= gf256_mul(coefficient, byte);
                    residual[2][offset] ^= gf256_mul(squared, byte);
                }
            }
            None => missing.push(index),
        }
    }
    let count = missing.len();
    if !(1..=3).contains(&count) {
        return Err("triple parity requires one to three missing shards");
    }

    // Invert the small Vandermonde coefficient matrix once per stripe.
    // Its first count columns describe the unknown data bytes and its
    // trailing columns become the inverse after elimination.
    let mut matrix = [[0u8; 6]; 3];
    for row in 0..count {
        for (col, &missing_index) in missing.iter().enumerate() {
            matrix[row][col] = gf256_pow(parity_coefficient(missing_index), row as u16);
        }
        matrix[row][count + row] = 1;
    }
    for col in 0..count {
        let pivot = (col..count)
            .find(|&row| matrix[row][col] != 0)
            .ok_or("singular triple parity coefficients")?;
        matrix.swap(col, pivot);
        let inverse = gf256_pow(matrix[col][col], 254);
        for slot in &mut matrix[col] {
            *slot = gf256_mul(*slot, inverse);
        }
        for row in 0..count {
            if row == col {
                continue;
            }
            let factor = matrix[row][col];
            let pivot_row = matrix[col];
            for (target, &pivot_value) in matrix[row][..count * 2]
                .iter_mut()
                .zip(&pivot_row[..count * 2])
            {
                *target ^= gf256_mul(factor, pivot_value);
            }
        }
    }

    let mut recovered = (0..count).map(|_| vec![0u8; width]).collect::<Vec<_>>();
    if width < 1024 {
        for (row, output) in recovered.iter_mut().enumerate() {
            for (offset, value) in output.iter_mut().enumerate() {
                for equation in 0..count {
                    *value ^= gf256_mul(matrix[row][count + equation], residual[equation][offset]);
                }
            }
        }
    } else {
        // Inverse matrix coefficients are fixed for the whole stripe.
        // Tabulate GF256 products once instead of multiplying every byte.
        let mut products = [[[0u8; 256]; 3]; 3];
        for row in 0..count {
            for equation in 0..count {
                let factor = matrix[row][count + equation];
                products[row][equation] =
                    std::array::from_fn(|byte| gf256_mul(factor, byte as u8));
            }
        }
        for (row, output) in recovered.iter_mut().enumerate() {
            for (offset, value) in output.iter_mut().enumerate() {
                for equation in 0..count {
                    *value ^= products[row][equation]
                        [usize::from(residual[equation][offset])];
                }
            }
        }
    }

    // The unused parity equations are independent checksums when fewer than
    // three erasures occurred. They catch inconsistent supplied parity data.
    for (equation, syndrome) in residual.iter().enumerate().skip(count) {
        for offset in 0..width {
            let mut actual = 0u8;
            for (value, &missing_index) in recovered.iter().zip(&missing) {
                let coefficient = parity_coefficient(missing_index);
                actual ^= gf256_mul(
                    gf256_pow(coefficient, equation as u16),
                    value[offset],
                );
            }
            if actual != syndrome[offset] {
                return Err("triple parity syndromes disagree");
            }
        }
    }
    for (data, &index) in recovered.iter_mut().zip(&missing) {
        let length = original_lengths[index];
        if data[length..].iter().any(|&byte| byte != 0) {
            return Err("nonzero triple parity padding");
        }
        data.truncate(length);
    }
    Ok(missing.into_iter().zip(recovered).collect())
}

#[cfg(test)]
mod triple_parity_tests {
    use super::*;

    #[test]
    fn all_one_two_and_three_erasures_reconstruct_exact_bytes() {
        let originals: [&[u8]; 6] = [
            b"short", b"", b"longer-payload", b"abc", b"binary\x00\xff", b"z"
        ];
        let parity = encode_triple_parity(&originals).unwrap();
        let lengths = originals.iter().map(|part| part.len()).collect::<Vec<_>>();
        for mask in 1usize..(1 << originals.len()) {
            if mask.count_ones() > 3 {
                continue;
            }
            let damaged = originals
                .iter()
                .enumerate()
                .map(|(index, &data)| (mask & (1 << index) == 0).then_some(data))
                .collect::<Vec<_>>();
            let result = recover_triple_erasure(&damaged, &lengths, &parity).unwrap();
            assert_eq!(result.len(), mask.count_ones() as usize);
            for (index, restored) in result {
                assert_eq!(restored, originals[index]);
            }
        }
    }

    #[test]
    fn wide_three_erasure_reconstruction_matches_exact_sources() {
        let sources = (0..6usize)
            .map(|shard| {
                (0..(4096 + shard * 31))
                    .map(|offset| (offset * 29 + shard * 71) as u8)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let parts = sources.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let lengths = sources.iter().map(Vec::len).collect::<Vec<_>>();
        let parity = encode_triple_parity(&parts).unwrap();
        for missing in [[0usize, 2usize, 5usize], [1usize, 3usize, 4usize]] {
            let available = parts
                .iter()
                .enumerate()
                .map(|(index, &part)| (!missing.contains(&index)).then_some(part))
                .collect::<Vec<_>>();
            let recovered = recover_triple_erasure(&available, &lengths, &parity).unwrap();
            for (index, bytes) in recovered {
                assert_eq!(bytes, sources[index]);
            }
        }
    }

    #[test]
    fn rejects_corrupt_syndromes_lengths_and_excess_erasures() {
        let originals: [&[u8]; 4] = [b"one", b"two", b"longest", b""];
        let parity = encode_triple_parity(&originals).unwrap();
        let lengths = originals.iter().map(|part| part.len()).collect::<Vec<_>>();
        let damaged = [None, Some(originals[1]), Some(originals[2]), Some(originals[3])];
        let mut corrupt = parity.clone();
        corrupt.squared[0] ^= 1;
        assert!(recover_triple_erasure(&damaged, &lengths, &corrupt).is_err());
        assert!(recover_triple_erasure(&[None, None, None, None], &lengths, &parity).is_err());
        assert!(recover_triple_erasure(&damaged, &[3, 3, 7], &parity).is_err());
        assert!(recover_triple_erasure(&[Some(b"x"), None, None, None], &lengths, &parity).is_err());
        assert!(encode_triple_parity(&[b"only"]).is_err());
    }
}
