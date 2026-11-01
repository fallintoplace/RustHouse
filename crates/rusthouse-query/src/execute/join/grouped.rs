//! Streaming GROUP BY key, COUNT(*) over INNER JOIN matches.
use super::*;
use std::collections::hash_map::Entry;

const MAX_JOIN_GROUPS: usize = 100_000;
const MAX_JOIN_GROUP_BYTES: usize = 64 * 1024 * 1024;

fn group_value_bytes(value: &typed::Cell) -> usize {
    match value {
        typed::Cell::Text(bytes) => bytes.len().saturating_mul(2),
        typed::Cell::Uuid(value) => value.len().saturating_mul(2),
        _ => 0,
    }
}

// Keep a single mutable probe key while building the grouped JOIN hash.
// Only distinct dimension keys need their own retained Vec allocation.
fn build_grouped_join_hash(
    rows: &[Vec<typed::Cell>],
    indices: &[usize],
    any: bool,
) -> HashMap<Vec<typed::Key>, Bucket> {
    let mut hash = HashMap::with_capacity(initial_hash_capacity(rows.len()));
    let mut key = Vec::with_capacity(indices.len());
    for (row_index, row) in rows.iter().enumerate() {
        if !row_key_into(row, indices, &mut key) {
            continue;
        }
        if let Some(bucket) = hash.get_mut(key.as_slice()) {
            if !any {
                bucket.push(row_index);
            }
        } else {
            hash.insert(key.clone(), Bucket::One(row_index));
        }
    }
    hash
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    left_names: &[String],
    right_names: &[String],
    left_keys: &[usize],
    right_keys: &[usize],
    null_filter: Option<(usize, bool)>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let key_column = query
        .grouped_count
        .as_ref()
        .ok_or_else(|| invalid("JOIN group key missing"))?;
    let width = left_names.len();
    let index = resolve(query, key_column, left_names, right_names)?;
    let group_name = format!("{}.{}", key_column.relation, key_column.name);
    if query.limit == Some(0) {
        writeln!(output, "{group_name}\tcount")?;
        return Ok(());
    }
    let selected = [(index, group_name.clone(), None)];
    let right_meta = inspect_query_source(right_path)?;
    let keep = retained_build_columns(right_names, right_keys, &selected, null_filter, width);
    let right_rows = read_projected_build_rows(
        right_path,
        right_names,
        &keep,
        right_meta.rows,
        MAX_STREAM_BUILD_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    let hash = build_grouped_join_hash(&right_rows, right_keys, query.any);
    // Store one representative display Cell and an exact count per group.
    let mut groups: HashMap<typed::Key, (typed::Cell, u64)> = HashMap::new();
    let mut group_count = 0usize;
    let mut bytes = 0usize;
    if !hash.is_empty() {
        probe_inner(
            left_path,
            left_names,
            left_keys,
            &right_rows,
            &hash,
            ProbeOptions {
                materialize_rows: index < width,
                materialize_after: 0,
                stop_after: Some(usize::MAX),
                any: query.any,
                null_filter,
            },
            |left, right| {
                let value = if index < width {
                    &left[index]
                } else {
                    &right[index - width]
                };
                let key = value.key();
                match groups.entry(key) {
                    Entry::Occupied(mut occupied) => {
                        let count = &mut occupied.get_mut().1;
                        *count = count.checked_add(1).ok_or_else(|| {
                            invalid("JOIN grouped count overflow")
                        })?;
                    }
                    Entry::Vacant(vacant) => {
                        if group_count >= MAX_JOIN_GROUPS {
                            return Err(invalid("too many JOIN groups"));
                        }
                        let variable = group_value_bytes(value);
                        bytes = bytes
                            .checked_add(
                                std::mem::size_of::<(typed::Key, typed::Cell, u64)>()
                                    .saturating_add(variable),
                            )
                            .ok_or_else(|| invalid("JOIN group memory overflow"))?;
                        if bytes > MAX_JOIN_GROUP_BYTES {
                            return Err(invalid("JOIN grouped hash memory budget exceeded"));
                        }
                        group_count += 1;
                        vacant.insert((value.clone(), 1));
                    }
                }
                Ok(())
            },
        )?;
    }
    let mut rows = groups.into_values().collect::<Vec<_>>();
    let (desc, nulls) = query
        .order_by
        .first()
        .map(|(_, desc, nulls)| (*desc, *nulls))
        .unwrap_or((false, Some(false)));
    rows.sort_by(|(left, _), (right, _)| {
        typed::compare_order_cells(left, right, desc, nulls)
    });
    writeln!(output, "{group_name}\tcount")?;
    for (group, count) in rows
        .iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(usize::MAX))
    {
        typed::write_value(output, group)?;
        writeln!(output, "\t{count}")?;
    }
    Ok(())
}

#[cfg(test)]
mod grouped_join_hash_build_tests {
    use super::*;

    #[test]
    fn grouped_build_skips_nulls_reuses_text_and_preserves_any_semantics() {
        let rows = vec![
            vec![typed::Cell::Text(b"same".to_vec())],
            vec![typed::Cell::Null],
            vec![typed::Cell::Text(b"same".to_vec())],
            vec![typed::Cell::Text(b"another".to_vec())],
        ];
        let key = vec![typed::Key::Text(b"same".to_vec())];
        let all = build_grouped_join_hash(&rows, &[0], false);
        assert_eq!(all.len(), 2);
        assert_eq!(
            all.get(key.as_slice()).unwrap().iter().copied().collect::<Vec<_>>(),
            vec![0, 2],
        );
        let any = build_grouped_join_hash(&rows, &[0], true);
        assert_eq!(
            any.get(key.as_slice()).unwrap().iter().copied().collect::<Vec<_>>(),
            vec![0],
        );
    }
}
