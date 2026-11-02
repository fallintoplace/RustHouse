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
