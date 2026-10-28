//! Globally correct, bounded k-way merge of sorted integer shard prefixes.
use super::{MergeError, MAX_RESPONSE_BYTES};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IntegerKey {
    Signed(i128),
    Unsigned(u128),
}

fn parse_key(bytes: &[u8], unsigned: bool) -> Result<IntegerKey, MergeError> {
    let text = std::str::from_utf8(bytes).map_err(|_| MergeError::InvalidSortedScan)?;
    if unsigned {
        text.parse::<u128>()
            .map(IntegerKey::Unsigned)
            .map_err(|_| MergeError::InvalidSortedScan)
    } else {
        text.parse::<i128>()
            .map(IntegerKey::Signed)
            .map_err(|_| MergeError::InvalidSortedScan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cursor {
    key: IntegerKey,
    shard: usize,
    position: usize,
    descending: bool,
}
impl Ord for Cursor {
    fn cmp(&self, other: &Self) -> Ordering {
        let cmp = self.key.cmp(&other.key);
        (if self.descending { cmp } else { cmp.reverse() })
            .then_with(|| other.shard.cmp(&self.shard))
            .then_with(|| other.position.cmp(&self.position))
    }
}
impl PartialOrd for Cursor {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

/// Validate every shard before emitting output, then keep one cursor per peer.
pub(super) fn merge_sorted_integer_page(
    shards: &[(usize, Vec<u8>)],
    column: &str,
    kind: &str,
    descending: bool,
    offset: usize,
    limit: usize,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    let header = format!("{column}\n");
    let unsigned = kind.starts_with("UInt");
    let prefix = offset.checked_add(limit).ok_or(MergeError::CoordinatorLimit)?;
    let mut parsed = Vec::with_capacity(shards.len());
    for (cap, body) in shards {
        if *cap <= prefix || !body.ends_with(b"\n") || !body.starts_with(header.as_bytes()) {
            return Err(MergeError::InvalidSortedScan);
        }
        let mut rows = Vec::<(IntegerKey, &[u8])>::new();
        let mut previous = None;
        for line in body[header.len()..].split_inclusive(|&byte| byte == b'\n') {
            if line.is_empty() { continue; }
            let value = line.strip_suffix(b"\n").ok_or(MergeError::InvalidSortedScan)?;
            if value.contains(&b'\t') || value == b"NULL" {
                return Err(MergeError::InvalidSortedScan);
            }
            let key = parse_key(value, unsigned)?;
            if previous.is_some_and(|prior| if descending { prior < key } else { prior > key }) {
                return Err(MergeError::InvalidSortedScan);
            }
            previous = Some(key);
            rows.push((key, line));
            if rows.len() > prefix || rows.len() >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
        }
        parsed.push(rows);
    }
    if parsed.is_empty() { return Err(MergeError::InvalidSortedScan); }
    let mut heap = BinaryHeap::new();
    for (shard, rows) in parsed.iter().enumerate() {
        if let Some(&(key, _)) = rows.first() {
            heap.push(Cursor { key, shard, position: 0, descending });
        }
    }
    let mut output = header.into_bytes();
    let mut seen = 0usize;
    let mut returned = 0usize;
    while let Some(cursor) = heap.pop() {
        if seen >= offset && returned < limit {
            let row = parsed[cursor.shard][cursor.position].1;
            if returned >= max_rows || output.len().checked_add(row.len())
                .is_none_or(|size| size > MAX_RESPONSE_BYTES)
            {
                return Err(MergeError::CoordinatorLimit);
            }
            output.extend_from_slice(row);
            returned += 1;
        }
        seen += 1;
        if returned >= limit { break; }
        let position = cursor.position + 1;
        if let Some(&(key, _)) = parsed[cursor.shard].get(position) {
            heap.push(Cursor { key, shard: cursor.shard, position, descending });
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shard(cap: usize, data: &str) -> (usize, Vec<u8>) {
        (cap, data.as_bytes().to_vec())
    }

    #[test]
    fn global_integer_topk_merges_peer_prefixes_and_paginates() {
        let shards = [
            shard(10, "id\n1\n4\n7\n"),
            shard(10, "id\n2\n3\n8\n"),
            shard(10, "id\n0\n5\n9\n"),
        ];
        assert_eq!(
            merge_sorted_integer_page(&shards, "id", "UInt64", false, 2, 3, 10).unwrap(),
            b"id\n2\n3\n4\n"
        );
        let desc = [shard(10, "id\n9\n5\n1\n"), shard(10, "id\n8\n6\n2\n")];
        assert_eq!(
            merge_sorted_integer_page(&desc, "id", "Int64", true, 1, 3, 10).unwrap(),
            b"id\n8\n6\n5\n"
        );
    }

    #[test]
    fn rejects_unsorted_shard_rows_and_insufficient_caps() {
        assert_eq!(
            merge_sorted_integer_page(&[shard(10, "id\n3\n2\n")], "id", "UInt64", false, 0, 2, 10),
            Err(MergeError::InvalidSortedScan)
        );
        assert_eq!(
            merge_sorted_integer_page(&[shard(2, "id\n1\n2\n")], "id", "UInt64", false, 0, 2, 10),
            Err(MergeError::InvalidSortedScan)
        );
    }

    #[test]
    fn unsigned_merge_preserves_uint128_above_signed_range() {
        let shards = [
            shard(5, "id\n340282366920938463463374607431768211454\n"),
            shard(5, "id\n340282366920938463463374607431768211455\n"),
        ];
        assert_eq!(
            merge_sorted_integer_page(&shards, "id", "UInt128", true, 0, 2, 10).unwrap(),
            b"id\n340282366920938463463374607431768211455\n340282366920938463463374607431768211454\n"
        );
    }
}
