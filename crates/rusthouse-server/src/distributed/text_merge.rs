//! Exact global top-K for non-nullable String keys in TSV shard prefixes.
//! Validate every shard before emitting a page, but retain only one decoded
//! candidate key per shard during the merge.
use super::{MAX_RESPONSE_BYTES, MergeError};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(PartialEq, Eq)]
struct TextHead {
    key: Vec<u8>,
    shard: usize,
    start: usize,
    end: usize,
    descending: bool,
}

impl Ord for TextHead {
    fn cmp(&self, other: &Self) -> Ordering {
        let cmp = self.key.cmp(&other.key);
        (if self.descending { cmp } else { cmp.reverse() })
            .then_with(|| other.shard.cmp(&self.shard))
            .then_with(|| other.start.cmp(&self.start))
    }
}

impl PartialOrd for TextHead {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Decode one row without allocating the projected, non-key columns.
/// The same parser is used for full validation and for advancing a heap head.
fn next_text_row(
    body: &[u8],
    start: usize,
    columns: usize,
) -> Result<Option<(usize, Vec<u8>)>, MergeError> {
    if start == body.len() {
        return Ok(None);
    }
    let relative_end = body
        .get(start..)
        .ok_or(MergeError::InvalidSortedScan)?
        .iter()
        .position(|&byte| byte == b'\n')
        .ok_or(MergeError::InvalidSortedScan)?;
    let end = start
        .checked_add(relative_end + 1)
        .ok_or(MergeError::CoordinatorLimit)?;
    let mut fields = body[start..end - 1].split(|&byte| byte == b'\t');
    let first = fields.next().ok_or(MergeError::InvalidSortedScan)?;
    if first == br"\N" {
        return Err(MergeError::InvalidSortedScan);
    }
    rusthouse_format::validate_tsv_field(first).map_err(|_| MergeError::InvalidSortedScan)?;
    let key =
        rusthouse_format::decode_tsv_field(first).map_err(|_| MergeError::InvalidSortedScan)?;
    let mut count = 1usize;
    for field in fields {
        count += 1;
        if count > columns {
            return Err(MergeError::InvalidSortedScan);
        }
        if field != br"\N" {
            rusthouse_format::validate_tsv_field(field)
                .map_err(|_| MergeError::InvalidSortedScan)?;
        }
    }
    if count != columns {
        return Err(MergeError::InvalidSortedScan);
    }
    Ok(Some((end, key)))
}

pub(super) fn merge_sorted_text_rows_page(
    shards: &[(usize, Vec<u8>)],
    columns: &[&str],
    descending: bool,
    offset: usize,
    limit: usize,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    if !(1..=16).contains(&columns.len()) || shards.is_empty() {
        return Err(MergeError::InvalidSortedScan);
    }
    let prefix = offset
        .checked_add(limit)
        .ok_or(MergeError::CoordinatorLimit)?;
    let header = format!("{}\n", columns.join("\t"));
    let mut decoded_key_bytes = 0usize;

    // Check *all* rows, not only the requested page. A corrupt or truncated
    // tail must never be hidden by an early LIMIT at the coordinator.
    for (cap, body) in shards {
        if *cap <= prefix || !body.ends_with(b"\n") || !body.starts_with(header.as_bytes()) {
            return Err(MergeError::InvalidSortedScan);
        }
        let mut position = header.len();
        let mut previous: Option<Vec<u8>> = None;
        let mut rows = 0usize;
        while let Some((end, key)) = next_text_row(body, position, columns.len())? {
            if previous.as_ref().is_some_and(|prior| {
                if descending {
                    prior < &key
                } else {
                    prior > &key
                }
            }) {
                return Err(MergeError::InvalidSortedScan);
            }
            rows = rows.checked_add(1).ok_or(MergeError::CoordinatorLimit)?;
            if rows > prefix || rows >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            decoded_key_bytes = decoded_key_bytes
                .checked_add(key.len())
                .filter(|&bytes| bytes <= MAX_RESPONSE_BYTES)
                .ok_or(MergeError::CoordinatorLimit)?;
            previous = Some(key);
            position = end;
        }
    }

    // Each heap entry owns only its current key and two byte offsets. The
    // response buffers remain borrowed, even for a very large merge page.
    let mut heap = BinaryHeap::new();
    for (shard, (_, body)) in shards.iter().enumerate() {
        if let Some((end, key)) = next_text_row(body, header.len(), columns.len())? {
            heap.push(TextHead {
                key,
                shard,
                start: header.len(),
                end,
                descending,
            });
        }
    }
    let mut result = header.into_bytes();
    let mut seen = 0usize;
    let mut returned = 0usize;
    while let Some(head) = heap.pop() {
        if seen >= offset && returned < limit {
            let bytes = &shards[head.shard].1[head.start..head.end];
            if returned >= max_rows
                || result
                    .len()
                    .checked_add(bytes.len())
                    .is_none_or(|size| size > MAX_RESPONSE_BYTES)
            {
                return Err(MergeError::CoordinatorLimit);
            }
            result.extend_from_slice(bytes);
            returned += 1;
        }
        seen += 1;
        if returned >= limit {
            break;
        }
        if let Some((end, key)) = next_text_row(&shards[head.shard].1, head.end, columns.len())? {
            heap.push(TextHead {
                key,
                shard: head.shard,
                start: head.end,
                end,
                descending,
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_decoded_string_keys_in_global_order() {
        let shards = [
            (10, b"name\tid\nA\\n\t1\nc\t2\n".to_vec()),
            (10, b"name\tid\nb\t3\nd\t4\n".to_vec()),
        ];
        assert_eq!(
            merge_sorted_text_rows_page(&shards, &["name", "id"], false, 1, 2, 2).unwrap(),
            b"name\tid\nb\t3\nc\t2\n"
        );
    }

    #[test]
    fn detects_corrupt_order_and_nullable_keys() {
        let bad = [(10, b"name\nz\na\n".to_vec())];
        assert_eq!(
            merge_sorted_text_rows_page(&bad, &["name"], false, 0, 2, 2),
            Err(MergeError::InvalidSortedScan),
        );
        let missing = [(10, b"name\n\\N\n".to_vec())];
        assert_eq!(
            merge_sorted_text_rows_page(&missing, &["name"], false, 0, 1, 1),
            Err(MergeError::InvalidSortedScan),
        );
    }

    #[test]
    fn uses_stable_peer_order_for_equal_keys() {
        let shards = [
            (10, b"name\tid\na\t1\na\t2\n".to_vec()),
            (10, b"name\tid\na\t3\nb\t4\n".to_vec()),
        ];
        assert_eq!(
            merge_sorted_text_rows_page(&shards, &["name", "id"], false, 0, 3, 3).unwrap(),
            b"name\tid\na\t1\na\t2\na\t3\n"
        );
    }

    #[test]
    fn validates_trailing_rows_outside_the_requested_page() {
        let bad = [(10, b"name\nb\nc\na\n".to_vec())];
        assert_eq!(
            merge_sorted_text_rows_page(&bad, &["name"], false, 0, 1, 1),
            Err(MergeError::InvalidSortedScan),
        );
        let bad_field = [(10, b"name\tid\na\t1\nb\n".to_vec())];
        assert_eq!(
            merge_sorted_text_rows_page(&bad_field, &["name", "id"], false, 0, 1, 1),
            Err(MergeError::InvalidSortedScan),
        );
    }

    #[test]
    fn merges_large_sorted_pages_without_retaining_all_keys() {
        let shards = (0..8)
            .map(|shard| {
                let mut body = String::from("key\tpayload\n");
                for index in 0..1000 {
                    body.push_str(&format!("{index:04}-{shard}\tvalue\n"));
                }
                (2000, body.into_bytes())
            })
            .collect::<Vec<_>>();
        let page =
            merge_sorted_text_rows_page(&shards, &["key", "payload"], false, 3990, 20, 20).unwrap();
        let rows = std::str::from_utf8(&page).unwrap();
        assert_eq!(rows.lines().count(), 21);
        assert!(rows.starts_with("key\tpayload\n0498-6\tvalue\n"));
        assert!(rows.ends_with("0501-1\tvalue\n"));
    }
}
