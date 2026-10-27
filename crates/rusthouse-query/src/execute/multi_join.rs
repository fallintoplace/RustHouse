//! Bounded left-deep execution for chained joins over persistent tables.
use super::{ExecutionError, typed};
use crate::{JoinColumn, JoinSelect, JoinStep, JoinType};
use rusthouse_storage::inspect_query_source;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use super::join::{MAX_INPUT_ROWS, MAX_RESULT_ROWS, MAX_STREAM_BUILD_BYTES, read_rows, row_key};

const MAX_CHAIN_BYTES: usize = 64 * 1024 * 1024;
const MAX_CHAIN_COLUMNS: usize = 128;

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

struct Relation {
    alias: String,
    names: Vec<String>,
    types: Vec<String>,
    start: usize,
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
        return Err(invalid("JOIN produces more than 65536 rows"));
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

fn step_keys(
    step: &JoinStep,
    relations: &[Relation],
    left_width: usize,
) -> Result<(Vec<usize>, Vec<usize>), ExecutionError> {
    let mut left_keys = Vec::with_capacity(step.keys.len());
    let mut right_keys = Vec::with_capacity(step.keys.len());
    for (first, second) in &step.keys {
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

/// Execute a chain of three or more INNER joins. No output is written before
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
        });
    }

    let first_meta = inspect_query_source(&paths[0])?;
    let mut rows = read_rows(
        &paths[0],
        &relations[0].names,
        first_meta.rows,
        MAX_INPUT_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    let initial_step = JoinStep {
        table: query.right.clone(),
        alias: query.right_alias.clone(),
        kind: query.kind,
        any: query.any,
        keys: query.keys.clone(),
        using_keys: query.using_keys.clone(),
    };
    for (step_index, step) in std::iter::once(&initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        if step.kind != JoinType::Inner || step.any || !step.using_keys.is_empty() {
            return Err(invalid("chained JOIN currently supports only INNER JOIN ON"));
        }
        let right_relation = &relations[step_index + 1];
        let left_width = right_relation.start;
        let (left_keys, right_keys) = step_keys(step, &relations[..=step_index + 1], left_width)?;
        let metadata = inspect_query_source(&paths[step_index + 1])?;
        let right_rows = read_rows(
            &paths[step_index + 1],
            &right_relation.names,
            metadata.rows,
            MAX_INPUT_ROWS,
            MAX_STREAM_BUILD_BYTES,
        )?;
        let mut hash: HashMap<Vec<typed::Key>, Vec<usize>> =
            HashMap::with_capacity(right_rows.len());
        for (index, row) in right_rows.iter().enumerate() {
            if let Some(key) = row_key(row, &right_keys) {
                hash.entry(key).or_default().push(index);
            }
        }
        let mut next = Vec::new();
        let mut retained = 0usize;
        for row in &rows {
            if let Some(key) = row_key(row, &left_keys) {
                if let Some(matches) = hash.get(&key) {
                    for &match_index in matches {
                        let mut joined = Vec::with_capacity(left_width + right_relation.names.len());
                        joined.extend_from_slice(row);
                        joined.extend_from_slice(&right_rows[match_index]);
                        push_row(&mut next, joined, &mut retained)?;
                    }
                }
            }
        }
        rows = next;
    }

    if let Some((column, not_null)) = &query.where_null {
        let index = column_index(&relations, column)?;
        rows.retain(|row| matches!(row[index], typed::Cell::Null) != *not_null);
    }
    if let Some((column, descending, nulls_first)) = &query.order_by {
        let index = column_index(&relations, column)?;
        rows.sort_by(|left, right| {
            typed::compare_order_cells(&left[index], &right[index], *descending, *nulls_first)
        });
    }
    let mut selected = Vec::new();
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
                    ));
                }
            } else {
                selected.push((
                    column_index(&relations, column)?,
                    format!("{}.{}", column.relation, column.name),
                ));
            }
        }
    } else {
        for relation in &relations {
            for (index, name) in relation.names.iter().enumerate() {
                selected.push((
                    relation.start + index,
                    format!("{}.{}", relation.alias, name),
                ));
            }
        }
    }
    let header = selected
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>()
        .join("\t");
    writeln!(output, "{header}")?;
    for row in rows
        .iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (position, (index, _)) in selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            typed::write_value(output, &row[*index])?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}
