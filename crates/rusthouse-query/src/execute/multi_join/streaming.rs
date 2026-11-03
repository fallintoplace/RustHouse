//! Streaming, bounded-build execution for INNER join chains.
//! The fact relation is read in batches; only dimension-side build rows persist.

use super::*;
use rusthouse_storage::TypedQueryReader;
use std::collections::hash_map::Entry;

use super::super::join::read_projected_build_rows;

const MAX_STREAM_GROUPS: usize = 100_000;
const MAX_STREAM_GROUP_BYTES: usize = 64 * 1024 * 1024;

// Conservatively charge hash buckets, key vectors and owned payloads as
// well as the already-accounted retained dimension rows.
fn hash_key_bytes(key: &[typed::Key]) -> Result<usize, ExecutionError> {
    let base = std::mem::size_of::<Vec<typed::Key>>()
        .checked_add(96)
        .and_then(|bytes| {
            key.len()
                .checked_mul(std::mem::size_of::<typed::Key>())
                .and_then(|slots| bytes.checked_add(slots))
        })
        .ok_or_else(|| invalid("JOIN hash key memory overflow"))?;
    key.iter().try_fold(base, |total, part| {
        let bytes = match part {
            typed::Key::Text(value) => value.len(),
            typed::Key::Uuid(value) => value.len(),
            _ => 0,
        };
        total
            .checked_add(bytes)
            .ok_or_else(|| invalid("JOIN hash key memory overflow"))
    })
}

fn charge_hash_build(retained: &mut usize, bytes: usize) -> Result<(), ExecutionError> {
    *retained = retained
        .checked_add(bytes)
        .filter(|&total| total <= MAX_CHAIN_BYTES)
        .ok_or_else(|| invalid("JOIN hash-build memory budget exceeded"))?;
    Ok(())
}

struct Stage {
    rows: Vec<Vec<typed::Cell>>,
    hash: HashMap<Vec<typed::Key>, Bucket>,
    left_keys: Vec<usize>,
    any: bool,
}

#[derive(Default)]
struct OutputState {
    seen: u64,
    emitted: usize,
    groups: HashMap<Vec<typed::Key>, (Vec<typed::Cell>, u64)>,
    group_key: Vec<typed::Key>,
    group_bytes: usize,
}

struct OutputSpec<'a> {
    selected: &'a [(usize, String, Option<usize>)],
    /// Physical source types, including projected USING fallbacks.
    types: Vec<&'a str>,
    null_filter: Option<(usize, bool)>,
    group_indices: Option<Vec<usize>>,
    offset: usize,
    limit: Option<usize>,
}

fn visit<W: Write>(
    depth: usize,
    row: &mut Vec<typed::Cell>,
    stages: &[Stage],
    scratch_keys: &mut [Vec<typed::Key>],
    spec: &OutputSpec<'_>,
    state: &mut OutputState,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if spec.limit.is_some_and(|limit| state.emitted >= limit) {
        return Ok(());
    }
    if depth == stages.len() {
        if let Some((column, not_null)) = spec.null_filter {
            let is_null = matches!(row[column], typed::Cell::Null);
            if is_null == not_null {
                return Ok(());
            }
        }
        if let Some(indices) = &spec.group_indices {
            state.group_key.clear();
            for &index in indices {
                state.group_key.push(row[index].key());
            }
            if let Some((_, count)) = state.groups.get_mut(state.group_key.as_slice()) {
                *count = count.checked_add(1)
                    .ok_or_else(|| invalid("chained JOIN group count overflow"))?;
            } else {
                if state.groups.len() >= MAX_STREAM_GROUPS {
                    return Err(invalid("chained JOIN supports at most 100000 groups"));
                }
                let variable = indices.iter().try_fold(0usize, |total, &index| {
                    let len = match &row[index] {
                        typed::Cell::Text(value) => value.len(),
                        typed::Cell::Uuid(value) => value.len(),
                        _ => 0,
                    };
                    len.checked_mul(2).and_then(|bytes| total.checked_add(bytes))
                        .ok_or_else(|| invalid("chained JOIN grouped key size overflow"))
                })?;
                let fixed = indices.len().checked_mul(
                    std::mem::size_of::<typed::Key>() + std::mem::size_of::<typed::Cell>()
                ).and_then(|bytes| bytes.checked_add(
                    std::mem::size_of::<(Vec<typed::Key>, Vec<typed::Cell>, u64)>()
                )).ok_or_else(|| invalid("chained JOIN group size overflow"))?;
                state.group_bytes = state.group_bytes
                    .checked_add(fixed)
                    .and_then(|bytes| bytes.checked_add(variable))
                    .filter(|&bytes| bytes <= MAX_STREAM_GROUP_BYTES)
                    .ok_or_else(|| invalid("chained JOIN grouped memory budget exceeded"))?;
                state.groups.insert(
                    state.group_key.clone(),
                    (indices.iter().map(|&index| row[index].clone()).collect(), 1),
                );
            }
            return Ok(());
        }
        state.seen = state
            .seen
            .checked_add(1)
            .ok_or_else(|| invalid("JOIN matched-row count overflow"))?;
        // Output is written directly to the caller, not kept in an
        // intermediate row vector. No materialized-join result cap applies.
        if state.seen <= spec.offset as u64 {
            return Ok(());
        }
        for (position, (index, _, fallback)) in spec.selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            let source = if matches!(row[*index], typed::Cell::Null) {
                fallback.unwrap_or(*index)
            } else {
                *index
            };
            typed::write_typed_value(output, &row[source], spec.types[source])?;
        }
        output.write_all(b"\n")?;
        state.emitted += 1;
        return Ok(());
    }

    let stage = &stages[depth];
    // Preserve per-stage key buffers across millions of fact rows.
    if !row_key_into(row, &stage.left_keys, &mut scratch_keys[depth]) {
        return Ok(());
    }
    let Some(matches) = stage.hash.get(scratch_keys[depth].as_slice()) else {
        return Ok(());
    };
    for &matched in matches.iter().take(if stage.any { 1 } else { usize::MAX }) {
        let original_len = row.len();
        row.extend_from_slice(&stage.rows[matched]);
        let result = visit(depth + 1, row, stages, scratch_keys, spec, state, output);
        row.truncate(original_len);
        result?;
        if spec.limit.is_some_and(|limit| state.emitted >= limit) {
            break;
        }
    }
    Ok(())
}

pub(super) fn execute<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    relations: &[Relation],
    initial_step: &JoinStep,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if relations[0].names.is_empty() {
        return Err(invalid("JOIN requires nonempty table schemas"));
    }
    let selected = output_columns(query, relations, initial_step)?;
    let null_filter = query
        .where_null
        .as_ref()
        .map(|(column, not_null)| column_index(relations, column).map(|index| (index, *not_null)))
        .transpose()?;
    let spec = OutputSpec {
        selected: &selected,
        types: relations
            .iter()
            .flat_map(|relation| relation.types.iter().map(String::as_str))
            .collect(),
        null_filter,
        group_indices: query.grouped_count.as_ref().map(|_| {
            selected.iter().map(|(index, _, _)| *index).collect()
        }),
        offset: query.offset,
        limit: query.limit,
    };
    // Project all dimension columns needed by any later stage, output
    // expression, or final filter. Preserve the original logical indexes.
    let width = relations
        .last()
        .map(|relation| relation.start + relation.names.len())
        .unwrap_or(0);
    let mut referenced = vec![false; width];
    for (step_index, step) in std::iter::once(initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let relation = &relations[step_index + 1];
        let (left, right) = step_keys(step, &relations[..=step_index + 1], relation.start)?;
        for index in left {
            referenced[index] = true;
        }
        for index in right {
            referenced[relation.start + index] = true;
        }
    }
    for &(index, _, fallback) in &selected {
        referenced[index] = true;
        if let Some(index) = fallback {
            referenced[index] = true;
        }
    }
    if let Some((index, _)) = null_filter {
        referenced[index] = true;
    }
    let projections = relations
        .iter()
        .map(|relation| {
            let mut positions = (0..relation.names.len())
                .filter(|&index| referenced[relation.start + index])
                .collect::<Vec<_>>();
            if positions.is_empty() && !relation.names.is_empty() {
                positions.push(0);
            }
            positions
        })
        .collect::<Vec<_>>();

    let mut stages = Vec::with_capacity(relations.len() - 1);
    let mut retained = 0usize;
    for (step_index, step) in std::iter::once(initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let relation = &relations[step_index + 1];
        let (left_keys, right_keys) =
            step_keys(step, &relations[..=step_index + 1], relation.start)?;
        let available = MAX_CHAIN_BYTES.saturating_sub(retained);
        let right_rows = read_projected_build_rows(
            &paths[step_index + 1],
            &relation.names,
            &projections[step_index + 1],
            relation.rows,
            crate::join::MAX_STREAM_JOIN_BUILD_ROWS,
            available.min(MAX_STREAM_BUILD_BYTES),
        )?;
        for row in &right_rows {
            retained = retained
                .checked_add(retained_row_bytes(row)?)
                .ok_or_else(|| invalid("JOIN build size overflow"))?;
        }
        if retained > MAX_CHAIN_BYTES {
            return Err(invalid("JOIN exceeds its retained-row memory budget"));
        }
        // A skewed build can contain far fewer distinct keys than input
        // rows. Reserve a small table, then charge each retained key.
        let mut hash = HashMap::with_capacity(right_rows.len().min(4096));
        for (index, row) in right_rows.iter().enumerate() {
            if let Some(key) = row_key(row, &right_keys) {
                match hash.entry(key) {
                    Entry::Vacant(entry) => {
                        charge_hash_build(&mut retained, hash_key_bytes(entry.key())?)?;
                        entry.insert(Bucket::One(index));
                    }
                    Entry::Occupied(mut entry) if !step.any => {
                        // Count a duplicate index and conservative Vec growth
                        // overhead. ANY only needs its first matching index.
                        charge_hash_build(&mut retained, std::mem::size_of::<usize>() * 2)?;
                        entry.get_mut().push(index);
                    }
                    Entry::Occupied(_) => {}
                }
            }
        }
        stages.push(Stage {
            rows: right_rows,
            hash,
            left_keys,
            any: step.any,
        });
    }
    if spec.group_indices.is_none() {
        write_join_header(output, &selected)?;
    }
    // A build can contain physical rows but no legal JOIN keys (only NULL
    // or NaN). In an INNER chain that is as empty as a zero-row dimension,
    // so do not read or decode the potentially enormous fact table.
    if stages.iter().any(|stage| stage.hash.is_empty()) {
        if query.grouped_count.is_some() {
            for (index, (_, name, _)) in selected.iter().enumerate() {
                if index != 0 {
                    output.write_all(b"\t")?;
                }
                output.write_all(name.as_bytes())?;
            }
            writeln!(output, "\t{}", query.aggregate_alias.as_deref().unwrap_or("count"))?;
        }
        return Ok(());
    }
    let mut scratch_keys = stages
        .iter()
        .map(|stage| Vec::with_capacity(stage.left_keys.len()))
        .collect::<Vec<Vec<typed::Key>>>();
    // Reuse the same dependency projection for the fact scan.
    let fact_width = relations[0].names.len();
    let projected = &projections[0];
    let projected_names = projected
        .iter()
        .map(|&index| relations[0].names[index].clone())
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(&paths[0], &projected_names)?;
    let mut scanned = 0u64;
    let mut state = OutputState::default();
    // The recursive visitor restores fact row length after each match.
    // Retain this allocation across the entire scan.
    let mut row = vec![typed::Cell::Null; fact_width];
    while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            scanned = scanned
                .checked_add(1)
                .ok_or_else(|| invalid("JOIN input row count overflow"))?;
            for (position, &column) in projected.iter().enumerate() {
                row[column] = typed::cell(&batch[position], index);
            }
            visit(
                0,
                &mut row,
                &stages,
                &mut scratch_keys,
                &spec,
                &mut state,
                output,
            )?;
            if spec.limit.is_some_and(|limit| state.emitted >= limit) {
                return Ok(());
            }
        }
    }
    if scanned != relations[0].rows {
        return Err(invalid("JOIN input row count changed during scan"));
    }
    if let Some(group_columns) = &query.grouped_count {
        let ordering = if query.order_by.is_empty() {
            (0..group_columns.len()).map(|index| (Some(index), false, Some(false))).collect::<Vec<_>>()
        } else {
            query.order_by.iter().map(|(column, descending, nulls)| {
                let index = if column.is_count_order() {
                    None
                } else {
                    Some(group_columns.iter().position(|group| group == column)
                        .ok_or_else(|| invalid("grouped JOIN ORDER BY must use a key or count"))?)
                };
                Ok((index, *descending, *nulls))
            }).collect::<Result<Vec<_>, ExecutionError>>()?
        };
        let mut rows = state.groups.into_values().collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            for &(index, descending, nulls_first) in &ordering {
                let result = if let Some(index) = index {
                    typed::compare_order_cells(
                        &left.0[index], &right.0[index], descending, nulls_first,
                    )
                } else {
                    let count_order = left.1.cmp(&right.1);
                    if descending { count_order.reverse() } else { count_order }
                };
                if !result.is_eq() {
                    return result;
                }
            }
            std::cmp::Ordering::Equal
        });
        for (index, (_, name, _)) in selected.iter().enumerate() {
            if index != 0 {
                output.write_all(b"\t")?;
            }
            output.write_all(name.as_bytes())?;
        }
        writeln!(output, "\t{}", query.aggregate_alias.as_deref().unwrap_or("count"))?;
        for (values, count) in rows.iter().skip(query.offset).take(query.limit.unwrap_or(usize::MAX)) {
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\t")?;
                }
                let source = selected[index].0;
                typed::write_typed_value(output, value, spec.types[source])?;
            }
            writeln!(output, "\t{count}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "streaming/stream_output_limit_tests.rs"]
mod stream_output_limit_tests;

#[cfg(test)]
#[path = "streaming/streaming_hash_budget_tests.rs"]
mod streaming_hash_budget_tests;
