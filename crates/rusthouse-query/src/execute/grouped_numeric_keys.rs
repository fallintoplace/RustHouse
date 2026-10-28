//! Numeric aggregates grouped by multiple typed columns.
use super::typed::{self, Cell, Key};
use super::{AggregateState, ExecutionError};
use crate::{AggregateFunction, Select};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    columns: &[String],
    items: &[(AggregateFunction, String)],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    for column in columns {
        if !types.contains_key(column) {
            return Err(ExecutionError::Invalid(format!("unknown group column: {column}")));
        }
    }

    let mut names = columns.to_vec();
    for (_, column) in items {
        let kind = types.get(column).ok_or_else(|| {
            ExecutionError::Invalid(format!("unknown aggregate column: {column}"))
        })?;
        if matches!(typed::base_type(kind), "String" | "UUID") {
            return Err(ExecutionError::Invalid(
                "numeric aggregates require numeric columns".into(),
            ));
        }
        if !names.contains(column) {
            names.push(column.clone());
        }
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let column = predicate.column();
        if !names.iter().any(|name| name == column) {
            names.push(column.to_owned());
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }

    write!(output, "{}", columns.join("\t"))?;
    for (function, column) in items {
        write!(output, "\t{}({column})", function.name())?;
    }
    writeln!(output)?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let group_indices = columns
        .iter()
        .map(|column| indices[column])
        .collect::<Vec<_>>();
    let item_indices = items
        .iter()
        .map(|(_, column)| indices[column])
        .collect::<Vec<_>>();
    let mut by_key = HashMap::<Vec<Key>, usize>::new();
    let mut groups = Vec::<(Vec<Cell>, Vec<AggregateState>)>::new();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    // Reuse a probe buffer for existing groups instead of allocating a Vec
    // of Cells and a Vec of Keys for every matching input row.
    let mut probe_key = Vec::<Key>::with_capacity(group_indices.len());
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            probe_key.clear();
            for &index in &group_indices {
                probe_key.push(typed::cell(&batch[index], row).key());
            }
            let group = if let Some(&index) = by_key.get(probe_key.as_slice()) {
                index
            } else {
                if groups.len() >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 200000 groups".into(),
                    ));
                }
                let index = groups.len();
                let keys = group_indices
                    .iter()
                    .map(|&column| typed::cell(&batch[column], row))
                    .collect::<Vec<_>>();
                by_key.insert(std::mem::take(&mut probe_key), index);
                probe_key = Vec::with_capacity(group_indices.len());
                groups.push((
                    keys,
                    items
                        .iter()
                        .map(|(function, _)| AggregateState::new(*function))
                        .collect(),
                ));
                index
            };
            for (&index, state) in item_indices.iter().zip(groups[group].1.iter_mut()) {
                match typed::cell(&batch[index], row) {
                    Cell::Numeric(value) => state.update(value)?,
                    Cell::Null => {}
                    Cell::Text(_) | Cell::Uuid(_) => {
                        return Err(ExecutionError::Invalid(
                            "numeric aggregates require numeric columns".into(),
                        ));
                    }
                }
            }
        }
    }

    let terms = query
        .order_terms()
        .map(|term| {
            let index = columns
                .iter()
                .position(|column| column == &term.column)
                .expect("GROUP BY ORDER BY validated by parser");
            (index, term.descending, term.nulls_first)
        })
        .collect::<Vec<_>>();
    if !terms.is_empty() {
        groups.sort_by(|(left, _), (right, _)| {
            for &(index, descending, nulls_first) in &terms {
                let order = typed::compare_order_cells(
                    &left[index],
                    &right[index],
                    descending,
                    nulls_first,
                );
                if order != Ordering::Equal {
                    return order;
                }
            }
            Ordering::Equal
        });
    }

    for (keys, states) in groups
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (index, key) in keys.iter().enumerate() {
            if index != 0 {
                output.write_all(b"\t")?;
            }
            typed::write_typed_value(output, key, &types[&columns[index]])?;
        }
        for state in states {
            write!(output, "\t{}", state.render())?;
        }
        writeln!(output)?;
    }
    Ok(())
}
