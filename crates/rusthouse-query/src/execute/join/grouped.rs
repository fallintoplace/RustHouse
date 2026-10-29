//! Streaming GROUP BY key, COUNT(*) over INNER JOIN matches.
use super::*;
use std::collections::hash_map::Entry;

const MAX_JOIN_GROUPS: usize = 100_000;
const MAX_JOIN_GROUP_BYTES: usize = 64 * 1024 * 1024;

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
    let mut hash = HashMap::<Vec<typed::Key>, Bucket>::with_capacity(right_rows.len());
    for (row_index, row) in right_rows.iter().enumerate() {
        if let Some(key) = row_key(row, right_keys) {
            if query.any {
                hash.entry(key).or_insert(Bucket::One(row_index));
            } else {
                hash.entry(key)
                    .and_modify(|bucket| bucket.push(row_index))
                    .or_insert(Bucket::One(row_index));
            }
        }
    }
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
                        let variable = match value {
                            typed::Cell::Text(value) => value.len().saturating_mul(2),
                            typed::Cell::Uuid(value) => value.len().saturating_mul(2),
                            _ => 0,
                        };
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
    let order = query.order_by.first().map(|(_, desc, nulls)| (*desc, *nulls));
    rows.sort_by(|(left, _), (right, _)| {
        let (desc, nulls) = order.unwrap_or((false, Some(false)));
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
