//! Globally correct, bounded k-way merge of sorted integer shard prefixes.
use super::{MergeError, MAX_RESPONSE_BYTES};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IntegerKey {
    Signed(i128),
    Unsigned(u128),
}

fn parse_integer_sort_key(bytes: &[u8], unsigned: bool) -> Result<IntegerKey, MergeError> {
    let text = std::str::from_utf8(bytes).map_err(|_| MergeError::InvalidSortedScan)?;
    if text.starts_with('+')
        || (text.len() > 1 && text.starts_with('0'))
        || text.starts_with("-0")
    {
        return Err(MergeError::InvalidSortedScan);
    }
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
    Ok(Some((parse_integer_sort_key(value, unsigned)?, end)))
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
    let header_len = header.len();
    let unsigned = kind.starts_with("UInt");
    let prefix = offset.checked_add(limit).ok_or(MergeError::CoordinatorLimit)?;
    let mut first_keys = Vec::with_capacity(shards.len());
    for (cap, body) in shards {
        if *cap <= prefix || !body.ends_with(b"\n") || !body.starts_with(header.as_bytes()) {
            return Err(MergeError::InvalidSortedScan);
        }
        let data = &body[header_len..];
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
        let body = &shards[cursor.shard].1[header_len..];
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


#[cfg(test)]
#[path = "sorted_merge/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sorted_merge/large_sorted_merge_tests.rs"]
mod large_sorted_merge_tests;

#[cfg(test)]
#[path = "sorted_merge/cursor_boundary_regression_tests.rs"]
mod cursor_boundary_regression_tests;
