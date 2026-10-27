//! Bounded multi-key numeric aggregation over immutable typed column batches.
use super::typed::{self, Cell, Key};
use super::{AggregateState, ExecutionError};
use crate::{AggregateFunction, Select};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{HashMap, hash_map::Entry};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 100_000;

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
    let mut names = columns.to_vec();
    for column in columns {
        if !types.contains_key(column) {
            return Err(ExecutionError::Invalid(format!("unknown group column: {column}")));
        }
    }
    for (_, column) in items {
        let data_type = types.get(column).ok_or_else(|| {
            ExecutionError::Invalid(format!("unknown aggregate column: {column}"))
        })?;
        if matches!(typed::base_type(data_type), "String" | "UUID") {
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
        if !names.contains(&predicate.column().to_owned()) {
            names.push(predicate.column().to_owned());
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let conditions = std::iter::once(&query.predicates)
        .chain(query.or_predicates.iter())
        .map(|predicates| typed::compile(predicates, &indices, &types))
        .collect::<Result<Vec<_>, _>>()?;

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
    let mut groups: Vec<(Vec<Cell>, Vec<AggregateState>)> = Vec::new();
    let mut by_key: HashMap<Vec<Key>, usize> = HashMap::new();
    let mut reader = TypedQueryReader::open(part, &names)?;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let values = group_indices
                .iter()
                .map(|&index| typed::cell(&batch[index], row))
                .collect::<Vec<_>>();
            let keys = values.iter().map(Cell::key).collect::<Vec<_>>();
            let index = match by_key.entry(keys) {
                Entry::Occupied(entry) => *entry.get(),
                Entry::Vacant(entry) => {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 100000 groups".into(),
                        ));
                    }
                    entry.insert(groups.len());
                    groups.push((
                        values,
                        items
                            .iter()
                            .map(|(function, _)| AggregateState::new(*function))
                            .collect(),
                    ));
                    groups.len() - 1
                }
            };
            for (&column, state) in item_indices.iter().zip(groups[index].1.iter_mut()) {
                match typed::cell(&batch[column], row) {
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
    if query.order_by.is_some() {
        let order_terms = query
            .order_terms()
            .map(|term| {
                let index = columns
                    .iter()
                    .position(|column| column == &term.column)
                    .expect("ORDER BY validated during parsing");
                (index, term.descending, term.nulls_first)
            })
            .collect::<Vec<_>>();
        groups.sort_by(|(left, _), (right, _)| {
            for &(index, descending, nulls_first) in &order_terms {
                let cmp = typed::compare_order_cells(
                    &left[index],
                    &right[index],
                    descending,
                    nulls_first,
                );
                if cmp != Ordering::Equal {
                    return cmp;
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
        for (index, value) in keys.iter().enumerate() {
            if index != 0 {
                output.write_all(b"\t")?;
            }
            typed::write_typed_value(output, value, &types[&columns[index]])?;
        }
        for state in states {
            write!(output, "\t{}", state.render())?;
        }
        writeln!(output)?;
    }
    Ok(())
}
