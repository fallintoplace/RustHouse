use super::*;

/// Ordinary counts sum across shards; DISTINCT needs global key de-duplication.
/// Only exact integer SUM is decomposable without changing the result type.
/// GROUP BY counts can be reduced by key across disjoint read-only shards.
/// A global HAVING, ORDER BY or LIMIT would require different planning.
/// Remove pagination before sending a scan to shards. Sending the original
/// LIMIT/OFFSET would drop rows on each shard and produce incorrect results.
/// Accept only a prefix that reparses to the exact unpaged query AST.
pub(super) fn merge_distinct_counts(shards: &[(usize, Vec<u8>)], column: &str) -> Result<Vec<u8>, MergeError> {
    const MAX_KEYS: usize = 100_000;
    let expected_header = format!("{column}\n");
    let mut values: HashSet<Cow<'_, [u8]>> = HashSet::new();
    for (cap, body) in shards {
        if !body.ends_with(b"\n") || !body.starts_with(expected_header.as_bytes()) {
            return Err(MergeError::InvalidDistinctCount);
        }
        let rows = &body[expected_header.len()..];
        let mut shard_rows = 0usize;
        // Slice::split includes the final empty segment. Remove the final
        // newline first, but preserve a genuine empty-string row.
        if !rows.is_empty() {
            let rows = &rows[..rows.len() - 1];
            for value in rows.split(|&b| b == b'\n') {
                shard_rows = shard_rows
                    .checked_add(1)
                    .ok_or(MergeError::CoordinatorLimit)?;
                if *cap == 0 || shard_rows >= *cap {
                    return Err(MergeError::ShardRowLimit);
                }
                if value != b"\\N" {
                    // Distinct values are logical TSV cells, not wire encodings.
                    // Borrow unescaped fields; allocate only when decoding escapes.
                    let normalized: Cow<'_, [u8]> = if value.contains(&b'\\') {
                        Cow::Owned(
                            rusthouse_format::decode_tsv_field(value)
                                .map_err(|_| MergeError::InvalidDistinctCount)?,
                        )
                    } else {
                        Cow::Borrowed(value)
                    };
                    if values.len() >= MAX_KEYS && !values.contains(normalized.as_ref()) {
                        return Err(MergeError::CoordinatorLimit);
                    }
                    values.insert(normalized);
                }
            }
        }
    }
    Ok(format!("count(DISTINCT {column})\n{}\n", values.len()).into_bytes())
}

/// Recover an exact physical type from the validated common shard schema.
pub(super) fn validated_schema_kind<'a>(schema: &'a [u8], column: &str) -> Option<&'a str> {
    let schema = std::str::from_utf8(schema).ok()?;
    let needle = format!("\"name\":\"{column}\",\"type\":\"");
    let (_, tail) = schema.split_once(&needle)?;
    let (kind, remaining) = tail.split_once('"')?;
    remaining.starts_with('}').then_some(kind)
}

/// Parse a UUID text key into its exact 128-bit identity. Hex-letter case
/// does not change UUID equality, so string hashing would be incorrect for
/// noncanonical but otherwise valid peer encodings.
pub(super) fn parse_uuid_wire(raw: &[u8]) -> Option<u128> {
    if raw.len() != 36 {
        return None;
    }
    let mut key = 0u128;
    for (index, &byte) in raw.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if byte != b'-' {
                return None;
            }
            continue;
        }
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        key = (key << 4) | u128::from(nibble);
    }
    Some(key)
}

pub(super) fn merge_distinct_uuid(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    nullable: bool,
) -> Result<Vec<u8>, MergeError> {
    const MAX_KEYS: usize = 100_000;
    let header = format!("{column}\n");
    let mut keys = HashSet::<u128>::new();
    for (cap, body) in shards {
        if !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidDistinctCount);
        }
        let rows = &body[header.len()..];
        if rows.is_empty() {
            continue;
        }
        let mut count = 0usize;
        for raw in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            count = count
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if *cap == 0 || count >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            if raw == br"\N" {
                if !nullable {
                    return Err(MergeError::InvalidDistinctCount);
                }
                continue;
            }
            let key = parse_uuid_wire(raw).ok_or(MergeError::InvalidDistinctCount)?;
            if keys.len() >= MAX_KEYS && !keys.contains(&key) {
                return Err(MergeError::CoordinatorLimit);
            }
            keys.insert(key);
        }
    }
    Ok(format!("count(DISTINCT {column})\n{}\n", keys.len()).into_bytes())
}

/// Deduplicate 64/128-bit integers as fixed-width values instead of owned
/// decimal TSV buffers. All peer schemas are checked before this reducer runs.
pub(super) fn merge_distinct_wide_integers(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    kind: &str,
) -> Result<Vec<u8>, MergeError> {
    const MAX_KEYS: usize = 100_000;
    let header = format!("{column}\n");
    let mut values = HashSet::<u128>::new();
    let kind = kind
        .strip_prefix("Nullable(")
        .and_then(|name| name.strip_suffix(')'))
        .unwrap_or(kind);
    if !matches!(kind, "Int64" | "UInt64" | "Int128" | "UInt128") {
        return Err(MergeError::InvalidDistinctCount);
    }
    for (cap, body) in shards {
        if !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidDistinctCount);
        }
        let rows = &body[header.len()..];
        if rows.is_empty() {
            continue;
        }
        let mut count = 0usize;
        for raw in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            count = count
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if *cap == 0 || count >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            if raw == br"\N" {
                continue;
            }
            let text = std::str::from_utf8(raw).map_err(|_| MergeError::InvalidDistinctCount)?;
            // Canonical wire integers exclude whitespace, leading zeroes and
            // signs on non-negative numbers. This also forbids ambiguous encodings.
            let key = match kind {
                "Int64" => text
                    .parse::<i64>()
                    .map(|number| number as u128)
                    .map_err(|_| MergeError::InvalidDistinctCount)?,
                "UInt64" => text
                    .parse::<u64>()
                    .map(u128::from)
                    .map_err(|_| MergeError::InvalidDistinctCount)?,
                "Int128" => text
                    .parse::<i128>()
                    .map(|number| number as u128)
                    .map_err(|_| MergeError::InvalidDistinctCount)?,
                _ => text
                    .parse::<u128>()
                    .map_err(|_| MergeError::InvalidDistinctCount)?,
            };
            let canonical = match kind {
                "Int64" => (key as i64).to_string(),
                "UInt64" => (key as u64).to_string(),
                "Int128" => (key as i128).to_string(),
                _ => key.to_string(),
            };
            if canonical != text {
                return Err(MergeError::InvalidDistinctCount);
            }
            if values.len() >= MAX_KEYS && !values.contains(&key) {
                return Err(MergeError::CoordinatorLimit);
            }
            values.insert(key);
        }
    }
    Ok(format!("count(DISTINCT {column})\n{}\n", values.len()).into_bytes())
}

/// Exact global DISTINCT for the complete signed or unsigned 8/16-bit domain.
///
/// Repeated values across shard-local DISTINCT scans are ordinary overlap, not
/// additional global keys. A fixed-size bitmap avoids hash allocations even
/// when a peer emits tens of thousands of distinct values.
pub(super) fn merge_distinct_bitmap16(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    kind: &str,
) -> Result<Vec<u8>, MergeError> {
    let width = match kind {
        "Int8" | "UInt8" | "Nullable(Int8)" | "Nullable(UInt8)" => 256,
        "Int16" | "UInt16" | "Nullable(Int16)" | "Nullable(UInt16)" => 65_536,
        _ => return Err(MergeError::InvalidDistinctCount),
    };
    let header = format!("{column}\n");
    let mut words = vec![0u64; width / 64];
    let mut distinct = 0u64;
    for (cap, body) in shards {
        if !body.starts_with(header.as_bytes()) || !body.ends_with(b"\n") {
            return Err(MergeError::InvalidDistinctCount);
        }
        let rows = &body[header.len()..];
        if rows.is_empty() {
            continue;
        }
        let mut count = 0usize;
        for raw in rows[..rows.len() - 1].split(|&byte| byte == b'\n') {
            count = count
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if *cap == 0 || count >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            if raw == br"\N" {
                continue;
            }
            let text = std::str::from_utf8(raw).map_err(|_| MergeError::InvalidDistinctCount)?;
            let index = match kind {
                "Int8" | "Nullable(Int8)" => text
                    .parse::<i8>()
                    .map_err(|_| MergeError::InvalidDistinctCount)? as u8 as usize,
                "UInt8" | "Nullable(UInt8)" => text
                    .parse::<u8>()
                    .map_err(|_| MergeError::InvalidDistinctCount)? as usize,
                "Int16" | "Nullable(Int16)" => text
                    .parse::<i16>()
                    .map_err(|_| MergeError::InvalidDistinctCount)? as u16 as usize,
                _ => text
                    .parse::<u16>()
                    .map_err(|_| MergeError::InvalidDistinctCount)? as usize,
            };
            let mask = 1u64 << (index & 63);
            let word = &mut words[index >> 6];
            if *word & mask == 0 {
                distinct += 1;
                *word |= mask;
            }
        }
    }
    Ok(format!("count(DISTINCT {column})\n{distinct}\n").into_bytes())
}

/// Exact global 32-bit DISTINCT union, bounded to 256 MiB of bitmap payload.
pub(super) fn merge_distinct_bitmap32(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    signed: bool,
) -> Result<Vec<u8>, MergeError> {
    const MAX_BLOCKS: usize = 32_768;
    let expected_header = format!("{column}\n");
    enum Block {
        // Most 32-bit prefixes have only a handful of distinct suffixes.
        // A sorted packed vector uses two bytes per suffix, without a
        // separate hash-table allocation for every sparse prefix.
        Sparse(Vec<u16>),
        Dense(Box<[u64; 1024]>),
    }
    // Switch to O(1) bitmap probes before sorted insertion becomes costly.
    const DENSE_THRESHOLD: usize = 256;
    let mut blocks = HashMap::<u16, Block>::new();
    let mut distinct = 0u64;
    for (cap, body) in shards {
        if !body.ends_with(b"\n") || !body.starts_with(expected_header.as_bytes()) {
            return Err(MergeError::InvalidDistinctCount);
        }
        let rows = &body[expected_header.len()..];
        if rows.is_empty() {
            continue;
        }
        let mut shard_rows = 0usize;
        for row in rows[..rows.len() - 1].split(|&b| b == b'\n') {
            shard_rows = shard_rows
                .checked_add(1)
                .ok_or(MergeError::CoordinatorLimit)?;
            if *cap == 0 || shard_rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            if row == b"\\N" {
                continue;
            }
            let value = std::str::from_utf8(row).map_err(|_| MergeError::InvalidDistinctCount)?;
            let key = if signed {
                value
                    .parse::<i32>()
                    .map_err(|_| MergeError::InvalidDistinctCount)? as u32
            } else {
                value
                    .parse::<u32>()
                    .map_err(|_| MergeError::InvalidDistinctCount)?
            };
            let prefix = (key >> 16) as u16;
            if blocks.len() >= MAX_BLOCKS && !blocks.contains_key(&prefix) {
                return Err(MergeError::CoordinatorLimit);
            }
            let suffix = (key & 0xffff) as u16;
            let block = blocks
                .entry(prefix)
                .or_insert_with(|| Block::Sparse(Vec::new()));
            let fresh = match block {
                Block::Sparse(values) => match values.binary_search(&suffix) {
                    Ok(_) => false,
                    Err(at) => {
                        values.insert(at, suffix);
                        if values.len() == DENSE_THRESHOLD {
                            let mut dense = Box::new([0u64; 1024]);
                            for &value in values.iter() {
                                let index = usize::from(value);
                                dense[index / 64] |= 1u64 << (index % 64);
                            }
                            *block = Block::Dense(dense);
                        }
                        true
                    }
                },
                Block::Dense(bits) => {
                    let index = usize::from(suffix);
                    let mask = 1u64 << (index % 64);
                    let word = &mut bits[index / 64];
                    let inserted = *word & mask == 0;
                    *word |= mask;
                    inserted
                }
            };
            if fresh {
                distinct = distinct.checked_add(1).ok_or(MergeError::CountOverflow)?;
            }
        }
    }
    Ok(format!("count(DISTINCT {column})\n{distinct}\n").into_bytes())
}

#[cfg(test)]
mod sparse_distinct_bitmap_regression_tests {
    use super::*;

    #[test]
    fn distributed_sparse_and_dense_prefixes_match_exact_set_count() {
        // Few suffixes per prefix, then a dense prefix past the promotion
        // threshold. Shards deliberately overlap and reverse key order.
        let mut keys = (0..32_000u32)
            .map(|n| (n << 16) | (n % 13))
            .collect::<Vec<_>>();
        keys.extend((0..1_500u32).map(|n| (7 << 16) | n));
        let expected = keys.iter().copied().collect::<HashSet<_>>().len();
        let first = format!(
            "key\n{}\n",
            keys.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        )
        .into_bytes();
        keys.reverse();
        let second = format!(
            "key\n{}\n",
            keys.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        )
        .into_bytes();
        let merged = merge_distinct_bitmap32(
            &[(keys.len() + 1, first), (keys.len() + 1, second)],
            "key",
            false,
        )
        .unwrap();
        assert_eq!(
            merged,
            format!("count(DISTINCT key)\n{expected}\n").into_bytes()
        );
    }
}

#[cfg(test)]
mod adaptive_distinct_bitmap_tests {
    use super::*;

    #[test]
    fn exact_sparse_prefixes_across_the_full_unsigned_domain() {
        let mut body = b"id\n".to_vec();
        for prefix in 0..32_768u32 {
            body.extend_from_slice(format!("{}\n", prefix << 16).as_bytes());
        }
        assert_eq!(
            merge_distinct_bitmap32(&[(40_000, body)], "id", false).unwrap(),
            b"count(DISTINCT id)\n32768\n"
        );
    }

    #[test]
    fn dense_promotion_keeps_exact_distinct_and_deduplicates_shards() {
        let mut body = b"id\n".to_vec();
        for suffix in 0..5_000u32 {
            body.extend_from_slice(format!("{}\n", (33u32 << 16) + suffix).as_bytes());
        }
        let shards = [(6_000, body.clone()), (6_000, body)];
        assert_eq!(
            merge_distinct_bitmap32(&shards, "id", false).unwrap(),
            b"count(DISTINCT id)\n5000\n"
        );
    }
}
/// Merge one strict 16 KiB register state per shard. Shards can overlap
/// arbitrarily: register-wise max computes the correct global HLL union.
pub(super) fn merge_uniq_shard_states(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    alias: Option<&str>,
) -> Result<Vec<u8>, MergeError> {
    let expected_header = format!("uniqState({column})\n");
    let mut states = Vec::with_capacity(shards.len());
    for (_, body) in shards {
        let state = body
            .strip_prefix(expected_header.as_bytes())
            .and_then(|bytes| bytes.strip_suffix(b"\n"))
            .ok_or(MergeError::InvalidUniqState)?;
        if state.contains(&b'\n') {
            return Err(MergeError::InvalidUniqState);
        }
        let state = std::str::from_utf8(state).map_err(|_| MergeError::InvalidUniqState)?;
        states.push(state);
    }
    let count =
        rusthouse_query::merge_uniq_states(&states).map_err(|_| MergeError::InvalidUniqState)?;
    let name = alias.map_or_else(|| format!("uniq({column})"), str::to_owned);
    Ok(format!("{name}\n{count}\n").into_bytes())
}
