//! Streaming, bounded-build execution for INNER join chains.
//! The fact relation is read in batches; only dimension-side build rows persist.

use super::*;
use rusthouse_storage::TypedQueryReader;
use std::collections::HashSet;

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

type GroupRow = (
    Vec<typed::Cell>,
    u64,
    Option<super::super::Number>,
    Option<HashSet<typed::Key>>,
);

#[derive(Default)]
struct OutputState {
    seen: u64,
    emitted: usize,
    scalar_sum: Option<super::super::Number>,
    formula_sums: Vec<Option<super::super::Number>>,
    groups: HashMap<Vec<typed::Key>, GroupRow>,
    group_key: Vec<typed::Key>,
    group_bytes: usize,
}

struct OutputSpec<'a> {
    selected: &'a [(usize, String, Option<usize>)],
    /// Physical source types, including projected USING fallbacks.
    types: Vec<&'a str>,
    null_filter: Option<(usize, bool)>,
    comparisons: &'a [(usize, crate::Operator, typed::Cell)],
    column_comparisons: &'a [(usize, crate::Operator, usize)],
    group_indices: Option<Vec<usize>>,
    sum: Option<&'a BoundSum>,
    scalar_sum: bool,
    formula: Option<&'a BoundFormula>,
    distinct_index: Option<usize>,
    row_filters: Option<&'a super::row_filters::RowFilters>,
    exists: &'a [super::exists::BoundExists],
    offset: usize,
    limit: Option<usize>,
}

fn evaluate_formula(
    formula: &BoundFormula,
    sums: &[Option<super::super::Number>],
) -> Result<Option<super::super::Number>, ExecutionError> {
    let row = sums
        .iter()
        .map(|value| value.map_or(typed::Cell::Null, typed::Cell::Numeric))
        .collect::<Vec<_>>();
    super::super::multi_aggregate::eval_row_arithmetic(&formula.expression, &row)
}

fn compare_sum_values(
    left: Option<super::super::Number>,
    right: Option<super::super::Number>,
    descending: bool,
    nulls_first: Option<bool>,
) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => {
            let order = super::super::multi_aggregate::compare_expression_numbers(left, right)
                .unwrap_or_else(|| super::super::compare_numbers(left, right));
            if descending { order.reverse() } else { order }
        }
        (left, right) => typed::compare_order_cells(
            &left.map_or(typed::Cell::Null, typed::Cell::Numeric),
            &right.map_or(typed::Cell::Null, typed::Cell::Numeric),
            descending,
            nulls_first,
        ),
    }
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
        if !spec.comparisons.iter().all(|(index, operator, literal)| {
            typed::matches_comparison(&row[*index], literal, *operator)
        }) || !spec
            .column_comparisons
            .iter()
            .all(|(left, operator, right)| {
                typed::matches_comparison(&row[*left], &row[*right], *operator)
            })
        {
            return Ok(());
        }
        if spec.row_filters.is_some_and(|filters| !filters.passes(row)) {
            return Ok(());
        }
        for filter in spec.exists {
            if !filter.passes(row)? {
                return Ok(());
            }
        }
        if let Some(formula) = spec.formula {
            for (index, sum) in formula.sums.iter().enumerate() {
                if let Some(value) =
                    super::super::multi_aggregate::eval_row_arithmetic(&sum.expression, row)?
                {
                    state.formula_sums[index] = Some(match state.formula_sums[index] {
                        Some(previous) => super::super::checked_aggregate_sum(previous, value)?,
                        None => value,
                    });
                }
            }
            return Ok(());
        }
        if spec.scalar_sum {
            if let Some(value) = super::super::multi_aggregate::eval_row_arithmetic(
                &spec.sum.expect("bound SUM").expression,
                row,
            )? {
                state.scalar_sum = Some(match state.scalar_sum {
                    Some(previous) => super::super::checked_aggregate_sum(previous, value)?,
                    None => value,
                });
            }
            return Ok(());
        }
        if let Some(indices) = &spec.group_indices {
            state.group_key.clear();
            for &index in indices {
                state.group_key.push(row[index].key());
            }
            let value = match spec.sum {
                Some(sum) => {
                    super::super::multi_aggregate::eval_row_arithmetic(&sum.expression, row)?
                }
                None => None,
            };
            if let Some((_, count, sum, _)) = state.groups.get_mut(state.group_key.as_slice()) {
                if let Some(value) = value {
                    *sum = Some(match *sum {
                        Some(previous) => super::super::checked_aggregate_sum(previous, value)?,
                        None => value,
                    });
                }
                if spec.distinct_index.is_none() {
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| invalid("chained JOIN group count overflow"))?;
                }
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
                    len.checked_mul(2)
                        .and_then(|bytes| total.checked_add(bytes))
                        .ok_or_else(|| invalid("chained JOIN grouped key size overflow"))
                })?;
                let fixed = indices
                    .len()
                    .checked_mul(
                        std::mem::size_of::<typed::Key>() + std::mem::size_of::<typed::Cell>(),
                    )
                    .and_then(|bytes| {
                        bytes.checked_add(std::mem::size_of::<(
                            Vec<typed::Key>,
                            Vec<typed::Cell>,
                            u64,
                            Option<super::super::Number>,
                            Option<HashSet<typed::Key>>,
                        )>())
                    })
                    .ok_or_else(|| invalid("chained JOIN group size overflow"))?;
                state.group_bytes = state
                    .group_bytes
                    .checked_add(fixed)
                    .and_then(|bytes| bytes.checked_add(variable))
                    .filter(|&bytes| bytes <= MAX_STREAM_GROUP_BYTES)
                    .ok_or_else(|| invalid("chained JOIN grouped memory budget exceeded"))?;
                state.groups.insert(
                    state.group_key.clone(),
                    (
                        indices.iter().map(|&index| row[index].clone()).collect(),
                        if spec.distinct_index.is_some() { 0 } else { 1 },
                        value,
                        spec.distinct_index.map(|_| HashSet::new()),
                    ),
                );
            }
            if let Some(index) = spec.distinct_index {
                if !matches!(row[index], typed::Cell::Null) {
                    let key = row[index].key();
                    let (_, count, _, distinct) = state
                        .groups
                        .get_mut(state.group_key.as_slice())
                        .expect("group exists");
                    let distinct = distinct.as_mut().expect("distinct set exists");
                    if !distinct.contains(&key) {
                        let charge = std::mem::size_of::<typed::Key>()
                            .saturating_mul(2)
                            .saturating_add(64);
                        state.group_bytes = state
                            .group_bytes
                            .checked_add(charge)
                            .filter(|&bytes| bytes <= MAX_STREAM_GROUP_BYTES)
                            .ok_or_else(|| {
                                invalid("chained JOIN grouped memory budget exceeded")
                            })?;
                        distinct.insert(key);
                        *count = count
                            .checked_add(1)
                            .ok_or_else(|| invalid("chained JOIN distinct count overflow"))?;
                    }
                }
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

#[allow(clippy::too_many_arguments)]
pub(super) fn execute<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    relations: &[Relation],
    initial_step: &JoinStep,
    comparisons: &[(usize, crate::Operator, typed::Cell)],
    column_comparisons: &[(usize, crate::Operator, usize)],
    sum: Option<&BoundSum>,
    formula: Option<&BoundFormula>,
    distinct_index: Option<usize>,
    row_filters: Option<&super::row_filters::RowFilters>,
    exists: &[super::exists::BoundExists],
    output: &mut W,
) -> Result<(), ExecutionError> {
    if relations[0].names.is_empty() {
        return Err(invalid("JOIN requires nonempty table schemas"));
    }
    let selected = if query.scalar_sum.is_some() || query.scalar_formula.is_some() {
        Vec::new()
    } else {
        output_columns(query, relations, initial_step)?
    };
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
        comparisons,
        column_comparisons,
        sum,
        scalar_sum: query.scalar_sum.is_some(),
        formula,
        distinct_index,
        row_filters,
        exists,
        group_indices: query
            .grouped_count
            .as_ref()
            .map(|_| selected.iter().map(|(index, _, _)| *index).collect()),
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
    for (index, _, _) in comparisons {
        referenced[*index] = true;
    }
    for (left, _, right) in column_comparisons {
        referenced[*left] = true;
        referenced[*right] = true;
    }
    if let Some(sum) = sum {
        for &index in &sum.indices {
            referenced[index] = true;
        }
    }
    if let Some(formula) = formula {
        for sum in &formula.sums {
            for &index in &sum.indices {
                referenced[index] = true;
            }
        }
    }
    if let Some(index) = distinct_index {
        referenced[index] = true;
    }
    if let Some(filters) = row_filters {
        for &index in &filters.indices {
            referenced[index] = true;
        }
    }
    for filter in exists {
        referenced[filter.equality] = true;
        if let Some(index) = filter.inequality {
            referenced[index] = true;
        }
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
        let mut hash =
            HashMap::<Vec<typed::Key>, Bucket>::with_capacity(right_rows.len().min(4096));
        let mut scratch_key = Vec::with_capacity(right_keys.len());
        for (index, row) in right_rows.iter().enumerate() {
            if !row_key_into(row, &right_keys, &mut scratch_key) {
                continue;
            }
            if let Some(bucket) = hash.get_mut(scratch_key.as_slice()) {
                if !step.any {
                    // Count a duplicate index and conservative Vec growth
                    // overhead. ANY only needs its first matching index.
                    charge_hash_build(&mut retained, std::mem::size_of::<usize>() * 2)?;
                    bucket.push(index);
                }
            } else {
                charge_hash_build(&mut retained, hash_key_bytes(&scratch_key)?)?;
                hash.insert(scratch_key.clone(), Bucket::One(index));
            }
        }
        stages.push(Stage {
            rows: right_rows,
            hash,
            left_keys,
            any: step.any,
        });
    }
    if spec.group_indices.is_none() && !spec.scalar_sum && formula.is_none() {
        write_join_header(output, &selected)?;
    }
    // A build can contain physical rows but no legal JOIN keys (only NULL
    // or NaN). In an INNER chain that is as empty as a zero-row dimension,
    // so do not read or decode the potentially enormous fact table.
    if stages.iter().any(|stage| stage.hash.is_empty()) {
        if query.grouped_count.is_some() {
            write_group_header(query, &selected, output)?;
        } else if let Some(formula) = formula {
            let value = evaluate_formula(formula, &vec![None; formula.sums.len()])?;
            write_scalar_sum(query, value, true, output)?;
        } else if spec.scalar_sum {
            write_scalar_sum(query, None, true, output)?;
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
    if let Some(formula) = formula {
        state.formula_sums.resize(formula.sums.len(), None);
    }
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
    if let Some(formula) = formula {
        let value = evaluate_formula(formula, &state.formula_sums)?;
        return write_scalar_sum(query, value, true, output);
    }
    if spec.scalar_sum {
        return write_scalar_sum(query, state.scalar_sum, true, output);
    }
    if let Some(group_columns) = &query.grouped_count {
        let ordering = if query.order_by.is_empty() {
            (0..group_columns.len())
                .map(|index| (Some(index), false, Some(false)))
                .collect::<Vec<_>>()
        } else {
            query
                .order_by
                .iter()
                .map(|(column, descending, nulls)| {
                    let index = if column.is_count_order() {
                        None
                    } else {
                        Some(
                            group_columns
                                .iter()
                                .position(|group| group == column)
                                .ok_or_else(|| {
                                    invalid("grouped JOIN ORDER BY must use a key or count")
                                })?,
                        )
                    };
                    Ok((index, *descending, *nulls))
                })
                .collect::<Result<Vec<_>, ExecutionError>>()?
        };
        let mut rows = state.groups.into_values().collect::<Vec<_>>();
        if let Some(having) = &query.grouped_sum_having {
            use super::super::Number;
            if having.value == "\\N" {
                rows.clear();
            } else {
                let value = if having.value.contains(['.', 'e', 'E']) {
                    Number::Float(
                        having
                            .value
                            .parse()
                            .map_err(|_| invalid("invalid JOIN HAVING number"))?,
                    )
                } else if having.value.starts_with('-') {
                    Number::Signed(
                        having
                            .value
                            .parse()
                            .map_err(|_| invalid("invalid JOIN HAVING number"))?,
                    )
                } else {
                    Number::Unsigned(
                        having
                            .value
                            .parse()
                            .map_err(|_| invalid("invalid JOIN HAVING number"))?,
                    )
                };
                rows.retain(|row| {
                    let Some(order) = row.2.and_then(|sum| {
                        super::super::multi_aggregate::compare_expression_numbers(sum, value)
                    }) else {
                        return false;
                    };
                    match having.operator {
                        crate::Operator::Eq => order.is_eq(),
                        crate::Operator::Ne => order.is_ne(),
                        crate::Operator::Lt => order.is_lt(),
                        crate::Operator::Le => order.is_le(),
                        crate::Operator::Gt => order.is_gt(),
                        crate::Operator::Ge => order.is_ge(),
                        _ => false,
                    }
                });
            }
        }
        let compare = |left: &GroupRow, right: &GroupRow| {
            for &(index, descending, nulls_first) in &ordering {
                let result = if let Some(index) = index {
                    typed::compare_order_cells(
                        &left.0[index],
                        &right.0[index],
                        descending,
                        nulls_first,
                    )
                } else {
                    if sum.is_some() {
                        compare_sum_values(left.2, right.2, descending, nulls_first)
                    } else {
                        let count_order = left.1.cmp(&right.1);
                        if descending {
                            count_order.reverse()
                        } else {
                            count_order
                        }
                    }
                };
                if !result.is_eq() {
                    return result;
                }
            }
            std::cmp::Ordering::Equal
        };
        let bounded = query
            .limit
            .and_then(|limit| query.offset.checked_add(limit))
            .filter(|&end| end < rows.len())
            .map(|end| super::super::sorted_group_positions(&rows, end, compare));
        write_group_header(query, &selected, output)?;
        let mut write_group = |values: &[typed::Cell],
                               count: u64,
                               sum: Option<super::super::Number>|
         -> Result<(), ExecutionError> {
            let metric = query.grouped_sum_position.unwrap_or(values.len());
            for index in 0..=values.len() {
                if index > 0 {
                    output.write_all(b"\t")?;
                }
                if index == metric {
                    if spec.sum.is_some() {
                        typed::write_value(
                            output,
                            &sum.map_or(typed::Cell::Null, typed::Cell::Numeric),
                        )?;
                    } else {
                        write!(output, "{count}")?;
                    }
                } else {
                    let key = if index < metric { index } else { index - 1 };
                    let source = selected[key].0;
                    typed::write_typed_value(output, &values[key], spec.types[source])?;
                }
            }
            writeln!(output)?;
            Ok(())
        };
        if let Some(positions) = bounded {
            for index in positions.into_iter().skip(query.offset) {
                let (values, count, sum, _) = &rows[index];
                write_group(values, *count, *sum)?;
            }
        } else {
            rows.sort_by(compare);
            for (values, count, sum, _) in rows
                .iter()
                .skip(query.offset)
                .take(query.limit.unwrap_or(usize::MAX))
            {
                write_group(values, *count, *sum)?;
            }
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
