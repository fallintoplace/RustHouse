//! Exact global top-K for String and Nullable(String) keys in TSV shard prefixes.
use super::{MAX_RESPONSE_BYTES, MergeError};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::ops::Range;

struct TextRow {
    range: Range<usize>,
    key: Vec<u8>,
}

#[derive(PartialEq, Eq)]
struct TextHead {
    key: Option<Vec<u8>>,
    shard: usize,
    index: usize,
    descending: bool,
    nulls_first: Option<bool>,
}

fn text_key_cmp(
    left: &Option<Vec<u8>>,
    right: &Option<Vec<u8>>,
    descending: bool,
    nulls_first: Option<bool>,
) -> Ordering {
    match (left, right, nulls_first) {
        (None, None, _) => Ordering::Equal,
        (None, Some(_), Some(true)) | (Some(_), None, Some(false)) => Ordering::Less,
        (None, Some(_), Some(false)) | (Some(_), None, Some(true)) => Ordering::Greater,
        _ => {
            let cmp = left.cmp(right);
            if descending { cmp.reverse() } else { cmp }
        }
    }
}

impl Ord for TextHead {
    fn cmp(&self, other: &Self) -> Ordering {
        text_key_cmp(&self.key, &other.key, self.descending, self.nulls_first)
            .reverse()
            .then_with(|| other.shard.cmp(&self.shard))
            .then_with(|| other.index.cmp(&self.index))
    }
}

impl PartialOrd for TextHead {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
pub(super) fn merge_sorted_text_rows_page(
    shards: &[(usize, Vec<u8>)],
    columns: &[&str],
    descending: bool,
    offset: usize,
    limit: usize,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    merge_sorted_text_rows_page_with_nulls(
        shards, columns, "String", descending, None, offset, limit, max_rows,
    )
}

pub(super) fn merge_sorted_text_rows_page_with_nulls(
    shards: &[(usize, Vec<u8>)],
    columns: &[&str],
    kind: &str,
    descending: bool,
    nulls_first: Option<bool>,
    offset: usize,
    limit: usize,
    max_rows: usize,
) -> Result<Vec<u8>, MergeError> {
    let nullable = match kind {
        "String" => false,
        "Nullable(String)" => true,
        _ => return Err(MergeError::InvalidSortedScan),
    };
    if !(1..=16).contains(&columns.len()) || shards.is_empty() {
        return Err(MergeError::InvalidSortedScan);
    }
    let prefix = offset
        .checked_add(limit)
        .ok_or(MergeError::CoordinatorLimit)?;
    let header = format!("{}\n", columns.join("\t"));
    let mut all_rows = Vec::with_capacity(shards.len());
    let mut decoded_key_bytes = 0usize;
    for (cap, body) in shards {
        if *cap <= prefix || !body.ends_with(b"\n") || !body.starts_with(header.as_bytes()) {
            return Err(MergeError::InvalidSortedScan);
        }
        let mut rows = Vec::new();
        let mut position = header.len();
        let mut previous: Option<Option<Vec<u8>>> = None;
        while position < body.len() {
            let end = body[position..]
                .iter()
                .position(|&byte| byte == b'\n')
                .ok_or(MergeError::InvalidSortedScan)?
                + position
                + 1;
            let fields = body[position..end - 1].split(|&byte| byte == b'\t');
            let mut first: Option<Option<Vec<u8>>> = None;
            let mut count = 0usize;
            for field in fields {
                count += 1;
                if count > columns.len() {
                    return Err(MergeError::InvalidSortedScan);
                }
                if field != br"\N" {
                    rusthouse_format::validate_tsv_field(field)
                        .map_err(|_| MergeError::InvalidSortedScan)?;
                }
                if count == 1 {
                    first = Some(if field == br"\N" {
                        if !nullable {
                            return Err(MergeError::InvalidSortedScan);
                        }
                        None
                    } else {
                        Some(
                            rusthouse_format::decode_tsv_field(field)
                                .map_err(|_| MergeError::InvalidSortedScan)?,
                        )
                    });
                }
            }
            if count != columns.len() {
                return Err(MergeError::InvalidSortedScan);
            }
            let key = first.ok_or(MergeError::InvalidSortedScan)?;
            if previous
                .as_ref()
                .is_some_and(|prior| text_key_cmp(prior, &key, descending, nulls_first).is_gt())
            {
                return Err(MergeError::InvalidSortedScan);
            }
            decoded_key_bytes = decoded_key_bytes
                .checked_add(key.as_ref().map_or(0, Vec::len))
                .filter(|&bytes| bytes <= MAX_RESPONSE_BYTES)
                .ok_or(MergeError::CoordinatorLimit)?;
            previous = Some(key.clone());
            rows.push(TextRow {
                range: position..end,
                key,
            });
            if rows.len() > prefix || rows.len() >= *cap {
                return Err(MergeError::ShardRowLimit);
            }
            position = end;
        }
        all_rows.push(rows);
    }

    let mut heap = BinaryHeap::new();
    for (shard, rows) in all_rows.iter().enumerate() {
        if let Some(first) = rows.first() {
            heap.push(TextHead {
                key: first.key.clone(),
                shard,
                index: 0,
                descending,
                nulls_first,
            });
        }
    }
    let mut result = header.into_bytes();
    let mut seen = 0usize;
    let mut returned = 0usize;
    while let Some(head) = heap.pop() {
        let row = &all_rows[head.shard][head.index];
        if seen >= offset && returned < limit {
            let bytes = &shards[head.shard].1[row.range.clone()];
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
        if let Some(next) = all_rows[head.shard].get(head.index + 1) {
            heap.push(TextHead {
                key: next.key.clone(),
                shard: head.shard,
                index: head.index + 1,
                descending,
                nulls_first,
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
}

#[cfg(test)]
mod nullable_text_topk_tests {
    use super::*;

    #[test]
    fn merges_nullable_strings_with_default_ascending_and_descending_order() {
        let ascending = [
            (10, b"name\tid\n\\N\t1\nb\t2\n".to_vec()),
            (10, b"name\tid\n\\N\t3\na\t4\n".to_vec()),
        ];
        assert_eq!(
            merge_sorted_text_rows_page_with_nulls(
                &ascending, &["name", "id"], "Nullable(String)", false, None, 0, 4, 4,
            ).unwrap(),
            b"name\tid\n\\N\t1\n\\N\t3\na\t4\nb\t2\n"
        );
        let descending = [
            (10, b"name\nid\n".to_vec()),
            (10, b"name\nb\n\\N\n".to_vec()),
        ];
        // A malformed field is rejected even when a short page could be returned.
        assert_eq!(
            merge_sorted_text_rows_page_with_nulls(
                &descending, &["name"], "Nullable(String)", true, None, 0, 1, 1,
            ),
            Err(MergeError::InvalidSortedScan)
        );
        let valid_descending = [
            (10, b"name\nz\n\\N\n".to_vec()),
            (10, b"name\ny\n\\N\n".to_vec()),
        ];
        assert_eq!(
            merge_sorted_text_rows_page_with_nulls(
                &valid_descending, &["name"], "Nullable(String)", true, None, 0, 4, 4,
            ).unwrap(),
            b"name\nz\ny\n\\N\n\\N\n"
        );
    }

    #[test]
    fn explicit_null_placement_is_not_reversed_with_descending() {
        let shards = [
            (10, b"name\tidx\na\t1\n\\N\t2\n".to_vec()),
            (10, b"name\tidx\nb\t3\n\\N\t4\n".to_vec()),
        ];
        assert_eq!(
            merge_sorted_text_rows_page_with_nulls(
                &shards, &["name", "idx"], "Nullable(String)", false, Some(false), 1, 3, 3,
            ).unwrap(),
            b"name\tidx\nb\t3\n\\N\t2\n\\N\t4\n"
        );
        let bad = [(10, b"name\n\\N\na\n".to_vec())];
        assert_eq!(
            merge_sorted_text_rows_page_with_nulls(
                &bad, &["name"], "Nullable(String)", false, Some(false), 0, 1, 1,
            ),
            Err(MergeError::InvalidSortedScan)
        );
        let first = [
            (10, b"name\n\\N\nz\n".to_vec()),
            (10, b"name\n\\N\ny\n".to_vec()),
        ];
        assert_eq!(
            merge_sorted_text_rows_page_with_nulls(
                &first, &["name"], "Nullable(String)", true, Some(true), 0, 4, 4,
            ).unwrap(),
            b"name\n\\N\n\\N\nz\ny\n"
        );
    }

    #[test]
    fn null_marker_is_distinct_from_a_literal_backslash_n_string() {
        let shards = [(10, b"name\n\\N\n\\\\N\n".to_vec())];
        assert_eq!(
            merge_sorted_text_rows_page_with_nulls(
                &shards, &["name"], "Nullable(String)", false, None, 0, 2, 2,
            ).unwrap(),
            b"name\n\\N\n\\\\N\n"
        );
        assert_eq!(
            merge_sorted_text_rows_page_with_nulls(
                &shards, &["name"], "String", false, None, 0, 2, 2,
            ),
            Err(MergeError::InvalidSortedScan)
        );
    }
}
