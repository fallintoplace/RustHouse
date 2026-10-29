//! Bounded left-deep execution for chained joins over persistent tables.
use super::{ExecutionError, typed};
use crate::{JoinColumn, JoinSelect, JoinStep, JoinType};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use super::join::{
    MAX_INPUT_ROWS, MAX_RESULT_ROWS, MAX_STREAM_BUILD_BYTES, multi_join_row_key as row_key,
    read_rows,
};

const MAX_CHAIN_BYTES: usize = crate::join::MAX_JOIN_CHAIN_BYTES;
const MAX_CHAIN_COLUMNS: usize = 128;

fn write_join_header<W: Write>(
    output: &mut W,
    selected: &[(usize, String, Option<usize>)],
) -> Result<(), ExecutionError> {
    for (index, (_, name, _)) in selected.iter().enumerate() {
        if index != 0 {
            output.write_all(b"\t")?;
        }
        output.write_all(name.as_bytes())?;
    }
    output.write_all(b"\n")?;
    Ok(())
}

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

/// Keep the common unique-key join bucket inline instead of allocating
/// a heap vector for every dimension key.
enum Bucket {
    One(usize),
    Many(Vec<usize>),
}

impl Bucket {
    fn push(&mut self, index: usize) {
        match self {
            Self::One(first) => *self = Self::Many(vec![*first, index]),
            Self::Many(indices) => indices.push(index),
        }
    }

    fn iter(&self) -> std::slice::Iter<'_, usize> {
        match self {
            Self::One(index) => std::slice::from_ref(index).iter(),
            Self::Many(indices) => indices.iter(),
        }
    }
}

struct Relation {
    alias: String,
    names: Vec<String>,
    types: Vec<String>,
    start: usize,
    rows: u64,
}

fn base_type(value: &str) -> &str {
    value
        .strip_prefix("Nullable(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value)
}

fn column_index(relations: &[Relation], column: &JoinColumn) -> Result<usize, ExecutionError> {
    let relation = relations
        .iter()
        .find(|relation| relation.alias == column.relation)
        .ok_or_else(|| invalid(format!("unknown JOIN qualifier: {}", column.relation)))?;
    relation
        .names
        .iter()
        .position(|name| name == &column.name)
        .map(|index| relation.start + index)
        .ok_or_else(|| {
            invalid(format!(
                "unknown JOIN column: {}.{}",
                column.relation, column.name
            ))
        })
}

fn column_type(relations: &[Relation], index: usize) -> &str {
    let relation = relations
        .iter()
        .rev()
        .find(|relation| index >= relation.start)
        .expect("column index validated");
    &relation.types[index - relation.start]
}

fn retained_row_bytes(row: &[typed::Cell]) -> Result<usize, ExecutionError> {
    let base = row
        .len()
        .checked_mul(std::mem::size_of::<typed::Cell>())
        .and_then(|value| value.checked_add(std::mem::size_of::<Vec<typed::Cell>>()))
        .ok_or_else(|| invalid("JOIN result size overflow"))?;
    row.iter().try_fold(base, |total, cell| {
        let additional = match cell {
            typed::Cell::Text(value) => value.len(),
            typed::Cell::Uuid(value) => value.len(),
            _ => 0,
        };
        total
            .checked_add(additional)
            .ok_or_else(|| invalid("JOIN result size overflow"))
    })
}

fn push_row(
    rows: &mut Vec<Vec<typed::Cell>>,
    row: Vec<typed::Cell>,
    retained: &mut usize,
) -> Result<(), ExecutionError> {
    if rows.len() >= MAX_RESULT_ROWS {
        return Err(invalid("JOIN produces more than 131072 rows"));
    }
    *retained = retained
        .checked_add(retained_row_bytes(&row)?)
        .ok_or_else(|| invalid("JOIN result size overflow"))?;
    if *retained > MAX_CHAIN_BYTES {
        return Err(invalid("JOIN exceeds its retained-row memory budget"));
    }
    rows.push(row);
    Ok(())
}

/// Resolve USING only when exactly one existing input supplies that key.
/// Ambiguous names must use qualified ON instead.
fn using_column_pair(relations: &[Relation], name: &str) -> Result<(usize, usize), ExecutionError> {
    let (right, earlier) = relations
        .split_last()
        .ok_or_else(|| invalid("JOIN USING has no relations"))?;
    let mut previous = None;
    for relation in earlier {
        if let Some(index) = relation.names.iter().position(|column| column == name) {
            if previous.replace(relation.start + index).is_some() {
                return Err(invalid(format!("ambiguous JOIN USING column: {name}")));
            }
        }
    }
    let left = previous.ok_or_else(|| invalid(format!("unknown JOIN USING column: {name}")))?;
    let right_index = right
        .names
        .iter()
        .position(|column| column == name)
        .ok_or_else(|| invalid(format!("unknown JOIN USING column: {name}")))?;
    Ok((left, right.start + right_index))
}

fn step_keys(
    step: &JoinStep,
    relations: &[Relation],
    left_width: usize,
) -> Result<(Vec<usize>, Vec<usize>), ExecutionError> {
    let count = step.keys.len() + step.using_keys.len();
    let mut left_keys = Vec::with_capacity(count);
    let mut right_keys = Vec::with_capacity(count);
    for name in &step.using_keys {
        let (left, right) = using_column_pair(relations, name)?;
        let left_type = column_type(relations, left);
        let right_type = column_type(relations, right);
        if base_type(left_type) != base_type(right_type) {
            return Err(invalid(format!(
                "JOIN key types differ: {left_type} and {right_type}"
            )));
        }
        left_keys.push(left);
        right_keys.push(right - left_width);
    }
    for (first, second) in step.keys.iter().take(if step.using_keys.is_empty() {
        usize::MAX
    } else {
        0
    }) {
        let first_index = column_index(relations, first)?;
        let second_index = column_index(relations, second)?;
        let (left, right) = if first_index < left_width && second_index >= left_width {
            (first_index, second_index)
        } else if second_index < left_width && first_index >= left_width {
            (second_index, first_index)
        } else {
            return Err(invalid(
                "JOIN ON must compare one existing relation to the newly joined table",
            ));
        };
        let left_type = column_type(relations, left);
        let right_type = column_type(relations, right);
        if base_type(left_type) != base_type(right_type) {
            return Err(invalid(format!(
                "JOIN key types differ: {left_type} and {right_type}"
            )));
        }
        left_keys.push(left);
        right_keys.push(right - left_width);
    }
    Ok((left_keys, right_keys))
}

/// Bind the output schema without opening or scanning any table rows.
fn output_columns(
    query: &JoinSelect,
    relations: &[Relation],
    initial_step: &JoinStep,
) -> Result<Vec<(usize, String, Option<usize>)>, ExecutionError> {
    // For unqualified *, the USING key appears once. With a RIGHT/FULL
    // join, fall back to the newly joined value when the old side is NULL.
    let mut omitted = HashMap::new();
    for (index, step) in std::iter::once(initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        for name in &step.using_keys {
            let (left, right) = using_column_pair(&relations[..=index + 1], name)?;
            omitted.insert(right, left);
        }
    }
    let mut selected: Vec<(usize, String, Option<usize>)> = Vec::new();
    if let Some(columns) = &query.columns {
        for column in columns {
            if column.name == "*" {
                let relation = relations
                    .iter()
                    .find(|relation| relation.alias == column.relation)
                    .ok_or_else(|| {
                        invalid(format!("unknown JOIN qualifier: {}", column.relation))
                    })?;
                for (index, name) in relation.names.iter().enumerate() {
                    selected.push((
                        relation.start + index,
                        format!("{}.{}", relation.alias, name),
                        None,
                    ));
                }
            } else {
                selected.push((
                    column_index(relations, column)?,
                    format!("{}.{}", column.relation, column.name),
                    None,
                ));
            }
        }
    } else {
        for relation in relations {
            for (index, name) in relation.names.iter().enumerate() {
                let absolute = relation.start + index;
                if let Some(&left) = omitted.get(&absolute) {
                    let existing = selected
                        .iter_mut()
                        .find(|(index, _, _)| *index == left)
                        .ok_or_else(|| invalid("JOIN USING output key was not projected"))?;
                    existing.2 = Some(absolute);
                } else {
                    selected.push((absolute, format!("{}.{}", relation.alias, name), None));
                }
            }
        }
    }
    Ok(selected)
}

/// Bound only the dimension hash builds. The fact table is scanned in typed
/// batches so all-inner COUNT chains can run over billions of probe rows.
struct CountStep {
    left_keys: Vec<usize>,
    any: bool,
    right_rows: Vec<Vec<typed::Cell>>,
    hash: HashMap<Vec<typed::Key>, Bucket>,
}

fn count_probe_chain(
    row: &mut Vec<typed::Cell>,
    steps: &[CountStep],
    index: usize,
) -> Result<u64, ExecutionError> {
    if index == steps.len() {
        return Ok(1);
    }
    let step = &steps[index];
    let Some(key) = row_key(row, &step.left_keys) else {
        return Ok(0);
    };
    let Some(matches) = step.hash.get(&key) else {
        return Ok(0);
    };
    let mut total = 0u64;
    for &right_index in matches.iter()
        .take(if step.any { 1 } else { usize::MAX })
    {
        let previous = row.len();
        row.extend_from_slice(&step.right_rows[right_index]);
        let increment = count_probe_chain(row, steps, index + 1)?;
        row.truncate(previous);
        total = total.checked_add(increment)
            .ok_or_else(|| invalid("JOIN COUNT overflow"))?;
    }
    Ok(total)
}

fn count_streaming_inner_chain<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    relations: &[Relation],
    initial_step: &JoinStep,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if query.offset != 0 {
        output.write_all(b"count\n")?;
        return Ok(());
    }
    let (steps, scan_indices) = prepare_stream_probe_steps(
        paths, relations, initial_step, &query.steps, &[],
    )?;
    let column_names = scan_indices.iter()
        .map(|&index| relations[0].names[index].clone())
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(&paths[0], &column_names)?;
    let mut count = 0u64;
    while let Some(batch) = reader.next_batch()? {
        for slot in 0..batch[0].len() {
            let mut fact = vec![typed::Cell::Null; relations[0].names.len()];
            for (read_index, &physical_index) in scan_indices.iter().enumerate() {
                fact[physical_index] = typed::cell(&batch[read_index], slot);
            }
            let increment = count_probe_chain(&mut fact, &steps, 0)?;
            count = count.checked_add(increment)
                .ok_or_else(|| invalid("JOIN COUNT overflow"))?;
        }
    }
    write!(output, "count\n{count}\n")?;
    Ok(())
}

fn prepare_stream_probe_steps(
    paths: &[PathBuf],
    relations: &[Relation],
    initial_step: &JoinStep,
    following: &[JoinStep],
    extra_fact_columns: &[usize],
) -> Result<(Vec<CountStep>, Vec<usize>), ExecutionError> {
    let mut steps = Vec::with_capacity(paths.len() - 1);
    let mut scan_indices = Vec::new();
    for (index, step) in std::iter::once(initial_step)
        .chain(following.iter()).enumerate()
    {
        let right = &relations[index + 1];
        let (left_keys, right_keys) = step_keys(
            step, &relations[..=index + 1], right.start
        )?;
        for &key in &left_keys {
            if key < relations[0].names.len() && !scan_indices.contains(&key) {
                scan_indices.push(key);
            }
        }
        let right_rows = read_rows(
            &paths[index + 1], &right.names, right.rows,
            MAX_INPUT_ROWS, MAX_STREAM_BUILD_BYTES,
        )?;
        let mut hash = HashMap::with_capacity(right_rows.len());
        for (row_index, row) in right_rows.iter().enumerate() {
            if let Some(key) = row_key(row, &right_keys) {
                hash.entry(key)
                    .and_modify(|bucket: &mut Bucket| bucket.push(row_index))
                    .or_insert(Bucket::One(row_index));
            }
        }
        steps.push(CountStep {
            left_keys, any: step.any, right_rows, hash,
        });
    }
    if scan_indices.is_empty() {
        scan_indices.push(0);
    }
    scan_indices.extend_from_slice(extra_fact_columns);
    scan_indices.sort_unstable();
    scan_indices.dedup();
    Ok((steps, scan_indices))
}

/// Visit one joined result at a time without building intermediate products.
/// Return false when the consumer has collected the requested output page.
fn visit_probe_chain<F>(
    row: &mut Vec<typed::Cell>,
    steps: &[CountStep],
    index: usize,
    emit: &mut F,
) -> Result<bool, ExecutionError>
where
    F: FnMut(&[typed::Cell]) -> Result<bool, ExecutionError>,
{
    if index == steps.len() {
        return emit(row);
    }
    let step = &steps[index];
    let Some(key) = row_key(row, &step.left_keys) else { return Ok(true); };
    let Some(matches) = step.hash.get(&key) else { return Ok(true); };
    for &right_index in matches.iter()
        .take(if step.any { 1 } else { usize::MAX })
    {
        let prior = row.len();
        row.extend_from_slice(&step.right_rows[right_index]);
        let keep_going = visit_probe_chain(row, steps, index + 1, emit)?;
        row.truncate(prior);
        if !keep_going {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Stream an unbounded fact source against bounded dimension hashes,
/// yielding only the requested unordered output page.
fn stream_selected_inner_chain<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    relations: &[Relation],
    initial_step: &JoinStep,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let selected = output_columns(query, relations, initial_step)?;
    let fact_width = relations[0].names.len();
    let extra = selected.iter().filter_map(|(index, _, fallback)| {
        let key = if *index < fact_width { Some(*index) } else { None };
        key.or_else(|| fallback.filter(|&index| index < fact_width))
    }).collect::<Vec<_>>();
    let (steps, indices) = prepare_stream_probe_steps(
        paths, relations, initial_step, &query.steps, &extra
    )?;
    let names = indices.iter().map(|&index| relations[0].names[index].clone())
        .collect::<Vec<_>>();
    let mut scan = TypedQueryReader::open(&paths[0], &names)?;
    write_join_header(output, &selected)?;
    let mut matched = 0usize;
    let end = query.offset.saturating_add(query.limit.unwrap_or(usize::MAX));
    'scan: while let Some(batch) = scan.next_batch()? {
        for row_index in 0..batch[0].len() {
            let mut row = vec![typed::Cell::Null; fact_width];
            for (read_index, &index) in indices.iter().enumerate() {
                row[index] = typed::cell(&batch[read_index], row_index);
            }
            let mut emit = |joined: &[typed::Cell]| -> Result<bool, ExecutionError> {
                if matched >= query.offset && matched < end {
                    for (position, (index, _, fallback)) in selected.iter().enumerate() {
                        if position != 0 {
                            output.write_all(b"\t")?;
                        }
                        let value = if matches!(joined[*index], typed::Cell::Null) {
                            fallback.map_or(&joined[*index], |other| &joined[other])
                        } else {
                            &joined[*index]
                        };
                        typed::write_value(output, value)?;
                    }
                    output.write_all(b"\n")?;
                }
                matched = matched.checked_add(1)
                    .ok_or_else(|| invalid("JOIN output row offset overflow"))?;
                Ok(matched < end)
            };
            if !visit_probe_chain(&mut row, &steps, 0, &mut emit)? {
                break 'scan;
            }
        }
    }
    Ok(())
}

/// Execute a chain of three or more bounded joins. No output is written before
/// all input validation, row-limit checks, and final ordering have succeeded.
pub fn execute_multi_join<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    output: &mut W,
) -> Result<(), ExecutionError> {
    if paths.len() != query.steps.len() + 2 {
        return Err(invalid("JOIN source count differs from its plan"));
    }
    let mut relations = Vec::with_capacity(paths.len());
    let mut width = 0usize;
    for (index, path) in paths.iter().enumerate() {
        let metadata = inspect_query_source(path)?;
        let names = metadata
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let types = metadata
            .columns
            .iter()
            .map(|column| column.data_type.clone())
            .collect::<Vec<_>>();
        let alias = match index {
            0 => query.left_alias.clone(),
            1 => query.right_alias.clone(),
            _ => query.steps[index - 2].alias.clone(),
        };
        width = width
            .checked_add(names.len())
            .ok_or_else(|| invalid("JOIN column count overflow"))?;
        if width > MAX_CHAIN_COLUMNS {
            return Err(invalid("JOIN supports at most 128 source columns"));
        }
        relations.push(Relation {
            alias,
            names,
            types,
            start: width - metadata.columns.len(),
            rows: metadata.rows,
        });
    }

    let initial_step = JoinStep {
        table: query.right.clone(),
        alias: query.right_alias.clone(),
        kind: query.kind,
        any: query.any,
        keys: query.keys.clone(),
        using_keys: query.using_keys.clone(),
    };
    // Validate the complete output schema without materializing rows.
    if query.limit == Some(0) || (query.aggregate.is_some() && query.offset > 0) {
        for (index, join) in std::iter::once(&initial_step)
            .chain(query.steps.iter())
            .enumerate()
        {
            let width = relations[index + 1].start;
            step_keys(join, &relations[..=index + 1], width)?;
        }
        if let Some((column, _)) = &query.where_null {
            column_index(&relations, column)?;
        }
        for (column, _, _) in &query.order_by {
            column_index(&relations, column)?;
        }
        if query.aggregate.is_some() {
            output.write_all(b"count\n")?;
        } else {
            let selected = output_columns(query, &relations, &initial_step)?;
            write_join_header(output, &selected)?;
        }
        return Ok(());
    }

    if query.aggregate.is_some() {
        return count_streaming_inner_chain(query, paths, &relations, &initial_step, output);
    }
    if query.limit.is_some_and(|limit| limit <= MAX_RESULT_ROWS)
        && query.order_by.is_empty()
        && query.where_null.is_none()
        && query.kind == JoinType::Inner
        && query.steps.iter().all(|step| step.kind == JoinType::Inner)
    {
        return stream_selected_inner_chain(
            query, paths, &relations, &initial_step, output
        );
    }

    let mut rows = read_rows(
        &paths[0],
        &relations[0].names,
        relations[0].rows,
        MAX_INPUT_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    for (step_index, step) in std::iter::once(&initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let right_relation = &relations[step_index + 1];
        let left_width = right_relation.start;
        let (left_keys, right_keys) = step_keys(step, &relations[..=step_index + 1], left_width)?;
        // INNER and LEFT cannot create rows from an empty intermediate.
        // Keep the source row limit even when its payload is not needed.
        if rows.is_empty() && matches!(step.kind, JoinType::Inner | JoinType::Left) {
            if right_relation.rows > MAX_INPUT_ROWS {
                return Err(invalid(format!(
                    "JOIN supports at most {MAX_INPUT_ROWS} input rows per table"
                )));
            }
            continue;
        }
        let right_rows = read_rows(
            &paths[step_index + 1],
            &right_relation.names,
            right_relation.rows,
            MAX_INPUT_ROWS,
            MAX_STREAM_BUILD_BYTES,
        )?;
        let mut hash: HashMap<Vec<typed::Key>, Bucket> = HashMap::with_capacity(right_rows.len());
        for (index, row) in right_rows.iter().enumerate() {
            if let Some(key) = row_key(row, &right_keys) {
                hash.entry(key)
                    .and_modify(|bucket| bucket.push(index))
                    .or_insert(Bucket::One(index));
            }
        }
        // COUNT(*) over an INNER join chain never needs to materialize the
        // final product. Count bucket multiplicities directly, avoiding the
        // intermediate result-row cap on high-fanout last joins.
        if query.aggregate.is_some() && step_index == query.steps.len() {
            let mut count = 0u64;
            for row in &rows {
                if let Some(key) = row_key(row, &left_keys) {
                    if let Some(matches) = hash.get(&key) {
                        let multiplicity = if step.any { 1 } else { matches.iter().len() };
                        let multiplicity = u64::try_from(multiplicity)
                            .map_err(|_| invalid("JOIN count overflow"))?;
                        count = count.checked_add(multiplicity)
                            .ok_or_else(|| invalid("JOIN count overflow"))?;
                    }
                }
            }
            write!(output, "count\n{count}\n")?;
            return Ok(());
        }
        // Most fact-to-dimension joins preserve the incoming row count.
        let mut next = Vec::with_capacity(rows.len().min(MAX_RESULT_ROWS));
        let mut retained = 0usize;
        let mut matched_right = matches!(step.kind, JoinType::Right | JoinType::Full)
            .then(|| vec![false; right_rows.len()]);
        for row in &rows {
            let mut matched = false;
            if let Some(key) = row_key(row, &left_keys) {
                if let Some(matches) = hash.get(&key) {
                    for &match_index in
                        matches
                            .iter()
                            .take(if step.any && step.kind != JoinType::Right {
                                1
                            } else {
                                usize::MAX
                            })
                    {
                        if step.any
                            && step.kind == JoinType::Right
                            && matched_right
                                .as_ref()
                                .is_some_and(|flags| flags[match_index])
                        {
                            continue;
                        }
                        matched = true;
                        if let Some(flags) = matched_right.as_mut() {
                            flags[match_index] = true;
                        }
                        let mut joined =
                            Vec::with_capacity(left_width + right_relation.names.len());
                        joined.extend_from_slice(row);
                        joined.extend_from_slice(&right_rows[match_index]);
                        push_row(&mut next, joined, &mut retained)?;
                    }
                }
            }
            if !matched && matches!(step.kind, JoinType::Left | JoinType::Full) {
                let mut joined = Vec::with_capacity(left_width + right_relation.names.len());
                joined.extend_from_slice(row);
                joined.resize(left_width + right_relation.names.len(), typed::Cell::Null);
                push_row(&mut next, joined, &mut retained)?;
            }
        }
        if let Some(matched_right) = matched_right {
            for (right_index, matched) in matched_right.into_iter().enumerate() {
                if !matched {
                    let mut joined = vec![typed::Cell::Null; left_width];
                    joined.extend_from_slice(&right_rows[right_index]);
                    push_row(&mut next, joined, &mut retained)?;
                }
            }
        }
        rows = next;
    }

    // An empty intermediate on an all-INNER chain yields a zero count.
    if query.aggregate.is_some() {
        output.write_all(b"count\n0\n")?;
        return Ok(());
    }

    if let Some((column, not_null)) = &query.where_null {
        let index = column_index(&relations, column)?;
        rows.retain(|row| matches!(row[index], typed::Cell::Null) != *not_null);
    }
    if !query.order_by.is_empty() {
        let ordering_columns = query
            .order_by
            .iter()
            .map(|(column, descending, nulls_first)| {
                column_index(&relations, column).map(|index| (index, *descending, *nulls_first))
            })
            .collect::<Result<Vec<_>, _>>()?;
        rows.sort_by(|left, right| {
            for (index, descending, nulls_first) in &ordering_columns {
                let ordering = typed::compare_order_cells(
                    &left[*index],
                    &right[*index],
                    *descending,
                    *nulls_first,
                );
                if !ordering.is_eq() {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        });
    }
    let selected = output_columns(query, &relations, &initial_step)?;
    write_join_header(output, &selected)?;
    for row in rows
        .iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (position, (index, _, fallback)) in selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            let value = if matches!(row[*index], typed::Cell::Null) {
                fallback.map_or(&row[*index], |right| &row[right])
            } else {
                &row[*index]
            };
            typed::write_value(output, value)?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod bucket_tests {
    use super::*;

    #[test]
    fn singleton_bucket_promotes_to_duplicate_matches_in_input_order() {
        let mut bucket = Bucket::One(3);
        assert_eq!(bucket.iter().copied().collect::<Vec<_>>(), vec![3]);
        bucket.push(7);
        bucket.push(9);
        assert_eq!(bucket.iter().copied().collect::<Vec<_>>(), vec![3, 7, 9]);
    }
}
