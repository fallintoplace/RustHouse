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
    next_position: usize,
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

/// Locate and parse only the next wire row. Positions are byte offsets in
/// the immutable shard body, not offsets into a materialized row array.
fn next_row(
    data: &[u8],
    start: usize,
    unsigned: bool,
) -> Result<Option<(IntegerKey, usize)>, MergeError> {
    if start == data.len() {
        return Ok(None);
    }
    let newline = data[start..].iter()
        .position(|&byte| byte == b'\n')
        .ok_or(MergeError::InvalidSortedScan)?;
    let end = start + newline + 1;
    let value = &data[start..end - 1];
    if value.contains(&b'\t') || value == b"NULL" {
        return Err(MergeError::InvalidSortedScan);
    }
    Ok(Some((parse_key(value, unsigned)?, end)))
}

/// Validate the full sorted shard streams before returning any rows.
/// Retain only one integer key and byte cursor per peer, rather than
/// materializing every shard row at the coordinator.
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
    let mut first_keys = Vec::with_capacity(shards.len());
    for (cap, body) in shards {
        if *cap <= prefix || !body.ends_with(b"\n") || !body.starts_with(header.as_bytes()) {
            return Err(MergeError::InvalidSortedScan);
        }
        let data = &body[header.len()..];
        let mut cursor = 0usize;
        let mut rows = 0usize;
        let mut previous = None;
        let mut first = None;
        while let Some((key, next)) = next_row(data, cursor, unsigned)? {
            if previous.is_some_and(|prior| if descending { prior < key } else { prior > key }) {
                return Err(MergeError::InvalidSortedScan);
            }
            if first.is_none() {
                first = Some((key, next));
            }
            previous = Some(key);
            rows += 1;
            if rows > prefix || rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            cursor = next;
        }
        first_keys.push(first);
    }
    if shards.is_empty() {
        return Err(MergeError::InvalidSortedScan);
    }
    let mut heap = BinaryHeap::new();
    for (shard, first) in first_keys.into_iter().enumerate() {
        if let Some((key, next_position)) = first {
            heap.push(Cursor {
                key, shard, position: 0, next_position, descending,
            });
        }
    }
    let mut output = header.into_bytes();
    let mut seen = 0usize;
    let mut returned = 0usize;
    while let Some(cursor) = heap.pop() {
        let body = &shards[cursor.shard].1[output_header_len(column)..];
        // The complete peer stream was already validated before heap merging.
        let end = cursor.next_position;
        if seen >= offset && returned < limit {
            let row = &body[cursor.position..end];
            if returned >= max_rows || output.len().checked_add(row.len())
                .is_none_or(|size| size > MAX_RESPONSE_BYTES)
            {
                return Err(MergeError::CoordinatorLimit);
            }
            output.extend_from_slice(row);
            returned += 1;
        }
        seen += 1;
        if returned >= limit {
            break;
        }
        if let Some((key, next_position)) = next_row(body, end, unsigned)? {
            heap.push(Cursor {
                key,
                shard: cursor.shard,
                position: end,
                next_position,
                descending,
            });
        }
    }
    Ok(output)
}

// The header is a validated single integer column and one newline.
fn output_header_len(column: &str) -> usize {
    column.len() + 1
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

#[cfg(test)]
mod large_sorted_merge_tests {
    use super::*;

    #[test]
    fn merges_large_high_fanout_prefixes_without_row_materialization() {
        let shards = (0..24).map(|shard| {
            let mut output = b"id\n".to_vec();
            for row in 0..2026 {
                output.extend_from_slice(format!("{}\n", row * 24 + shard).as_bytes());
            }
            (4096, output)
        }).collect::<Vec<_>>();
        let actual = merge_sorted_integer_page(&shards, "id", "UInt64", false, 1999, 27, 27).unwrap();
        let mut expected = b"id\n".to_vec();
        for value in 1999..2026 {
            expected.extend_from_slice(format!("{value}\n").as_bytes());
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn validates_all_shards_before_returning_even_when_page_is_empty() {
        let bad = [(5, b"id\n1\n".to_vec()), (5, b"id\n9\n3\n".to_vec())];
        assert_eq!(
            merge_sorted_integer_page(&bad, "id", "UInt64", false, 0, 0, 100),
            Err(MergeError::InvalidSortedScan)
        );
    }
}

#[cfg(test)]
mod cursor_boundary_regression_tests {
    use super::*;

    #[test]
    fn preserves_signed_duplicate_pages_across_shard_boundaries() {
        let shards = [
            (12, b"id\n-9\n-2\n-2\n7\n".to_vec()),
            (12, b"id\n-12\n-2\n0\n8\n".to_vec()),
            (12, b"id\n-10\n-2\n1\n99\n".to_vec()),
        ];
        assert_eq!(
            merge_sorted_integer_page(&shards, "id", "Int64", false, 2, 5, 5).unwrap(),
            b"id\n-9\n-2\n-2\n-2\n-2\n"
        );
        assert_eq!(
            merge_sorted_integer_page(&shards, "id", "Int64", false, 1, 0, 5).unwrap(),
            b"id\n"
        );
    }
}
