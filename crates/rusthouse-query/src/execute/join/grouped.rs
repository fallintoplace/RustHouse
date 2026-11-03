//! Streaming GROUP BY key, COUNT(*) over INNER and LEFT JOIN rows.
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
    let mut hash: HashMap<Vec<typed::Key>, Bucket> =
        HashMap::with_capacity(initial_hash_capacity(rows.len()));
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

// Match the inner grouped path's exact count and retained-memory limits.
fn add_group(
    groups: &mut HashMap<typed::Key, (typed::Cell, u64)>,
    value: &typed::Cell,
    group_count: &mut usize,
    bytes: &mut usize,
) -> Result<(), ExecutionError> {
    match groups.entry(value.key()) {
        Entry::Occupied(mut occupied) => {
            let count = &mut occupied.get_mut().1;
            *count = count
                .checked_add(1)
                .ok_or_else(|| invalid("JOIN grouped count overflow"))?;
        }
        Entry::Vacant(vacant) => {
            if *group_count >= MAX_JOIN_GROUPS {
                return Err(invalid("too many JOIN groups"));
            }
            let variable = group_value_bytes(value);
            *bytes = bytes
                .checked_add(
                    std::mem::size_of::<(typed::Key, typed::Cell, u64)>()
                        .saturating_add(variable),
                )
                .ok_or_else(|| invalid("JOIN group memory overflow"))?;
            if *bytes > MAX_JOIN_GROUP_BYTES {
                return Err(invalid("JOIN grouped hash memory budget exceeded"));
            }
            *group_count += 1;
            vacant.insert((value.clone(), 1));
        }
    }
    Ok(())
}

// Stream the preserved LEFT input even when the right hash is empty. Null
// join keys and missing matches produce one null-extended row per left row.
#[allow(clippy::too_many_arguments)]
fn probe_grouped_left<F>(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    hash: &HashMap<Vec<typed::Key>, Bucket>,
    group_index: usize,
    null_filter: Option<(usize, bool)>,
    any: bool,
    mut emit: F,
) -> Result<(), ExecutionError>
where
    F: FnMut(&typed::Cell) -> Result<(), ExecutionError>,
{
    let width = left_names.len();
    let mut retained = left_keys.to_vec();
    if group_index < width {
        retained.push(group_index);
    }
    if let Some((column, _)) = null_filter {
        if column < width {
            retained.push(column);
        }
    }
    retained.sort_unstable();
    retained.dedup();
    if retained.is_empty() {
        retained.push(0);
    }
    let names = retained
        .iter()
        .map(|&column| left_names[column].clone())
        .collect::<Vec<_>>();
    let key_indices = left_keys
        .iter()
        .map(|column| retained.binary_search(column).expect("JOIN key retained"))
        .collect::<Vec<_>>();
    let group_batch = (group_index < width)
        .then(|| retained.binary_search(&group_index).expect("group key retained"));
    let filter_batch = null_filter.and_then(|(column, _)| {
        (column < width).then(|| retained.binary_search(&column).expect("filter retained"))
    });
    let mut reader = TypedQueryReader::open(left_path, &names)?;
    let mut key = Vec::with_capacity(left_keys.len());
    let null = typed::Cell::Null;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            let left_group = group_batch.map(|column| typed::cell(&batch[column], row));
            let mut matched = false;
            if batch_key_into(&batch, row, &key_indices, &mut key) {
                if let Some(indices) = hash.get(key.as_slice()) {
                    for &right_index in indices.iter().take(if any { 1 } else { usize::MAX }) {
                        // ON matches determine outer-row preservation before WHERE.
                        matched = true;
                        if let Some((column, not_null)) = null_filter {
                            let is_null = if let Some(index) = filter_batch {
                                typed::is_null_at(&batch[index], row)
                            } else {
                                matches!(
                                    &right_rows[right_index][column - width],
                                    typed::Cell::Null
                                )
                            };
                            if is_null == not_null {
                                continue;
                            }
                        }
                        let value = if let Some(value) = left_group.as_ref() {
                            value
                        } else {
                            &right_rows[right_index][group_index - width]
                        };
                        emit(value)?;
                    }
                }
            }
            if !matched {
                // Every unmatched row has NULL in all right-side columns.
                if let Some((_, not_null)) = null_filter {
                    let is_null = if let Some(index) = filter_batch {
                        typed::is_null_at(&batch[index], row)
                    } else {
                        true
                    };
                    if is_null == not_null {
                        continue;
                    }
                }
                emit(left_group.as_ref().unwrap_or(&null))?;
            }
        }
    }
    Ok(())
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
    let group_type = if index < width {
        inspect_query_source(left_path)?.columns[index].data_type.clone()
    } else {
        inspect_query_source(right_path)?.columns[index - width].data_type.clone()
    };
    let group_name = format!("{}.{}", key_column.relation, key_column.name);
    if query.limit == Some(0) {
        writeln!(output, "{group_name}\t{}", query.aggregate_alias.as_deref().unwrap_or("count"))?;
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
    if query.kind == JoinType::Left {
        probe_grouped_left(
            left_path,
            left_names,
            left_keys,
            &right_rows,
            &hash,
            index,
            null_filter,
            query.any,
            |value| add_group(&mut groups, value, &mut group_count, &mut bytes),
        )?;
    } else if !hash.is_empty() {
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
                add_group(&mut groups, value, &mut group_count, &mut bytes)
            },
        )?;
    }
    let mut rows = groups.into_values().collect::<Vec<_>>();
    let (desc, nulls) = query
        .order_by
        .first()
        .map(|(_, desc, nulls)| (*desc, *nulls))
        .unwrap_or((false, Some(false)));
    let compare = |(left, _): &(typed::Cell, u64), (right, _): &(typed::Cell, u64)| {
        typed::compare_order_cells(left, right, desc, nulls)
    };
    // A paginated GROUP BY only needs its first OFFSET + LIMIT keys.
    // Preserve the original stable tie order through sorted_group_positions.
    let bounded = query
        .limit
        .and_then(|limit| query.offset.checked_add(limit))
        .filter(|&end| end < rows.len())
        .map(|end| super::super::sorted_group_positions(&rows, end, &compare));
    writeln!(output, "{group_name}\t{}", query.aggregate_alias.as_deref().unwrap_or("count"))?;
    if let Some(indices) = bounded {
        for index in indices.into_iter().skip(query.offset) {
            let (group, count) = &rows[index];
            typed::write_typed_value(output, group, &group_type)?;
            writeln!(output, "\t{count}")?;
        }
    } else {
        rows.sort_by(compare);
        for (group, count) in rows
            .iter()
            .skip(query.offset)
            .take(query.limit.unwrap_or(usize::MAX))
        {
            typed::write_typed_value(output, group, &group_type)?;
            writeln!(output, "\t{count}")?;
        }
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
            all.get(key.as_slice())
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![0, 2],
        );
        let any = build_grouped_join_hash(&rows, &[0], true);
        assert_eq!(
            any.get(key.as_slice())
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![0],
        );
    }
}

#[cfg(test)]
mod bounded_grouped_join_page_tests {
    use super::*;

    #[test]
    fn bounded_group_page_matches_stable_full_sort() {
        let rows = vec![
            (typed::Cell::Numeric(Number::Unsigned(5)), 2),
            (typed::Cell::Numeric(Number::Unsigned(8)), 4),
            (typed::Cell::Numeric(Number::Unsigned(5)), 6),
            (typed::Cell::Numeric(Number::Unsigned(1)), 1),
            (typed::Cell::Numeric(Number::Unsigned(9)), 3),
        ];
        for descending in [false, true] {
            let compare = |(a, _): &(typed::Cell, u64), (b, _): &(typed::Cell, u64)| {
                typed::compare_order_cells(a, b, descending, Some(false))
            };
            let mut baseline = (0..rows.len()).collect::<Vec<_>>();
            baseline.sort_by(|&a, &b| compare(&rows[a], &rows[b]));
            for end in 1..rows.len() {
                assert_eq!(
                    super::super::super::sorted_group_positions(&rows, end, &compare),
                    baseline[..end],
                );
            }
        }
    }
}
