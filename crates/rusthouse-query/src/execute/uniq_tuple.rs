//! Fixed 16 KiB composite distinct estimator over high-cardinality fact rows.
//! Typed keys are hashed field-by-field with type/length framing; no composite
//! String is materialized or stored for any input row.
use super::hyperloglog::HyperLogLog;
use super::predicates::{compile_numbers_predicates, count_monotonic_numbers, number_matches_all};
use super::typed;
use super::{ExecutionError, Number, write_header};
use crate::{Projection, Select};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

fn header(columns: &[String]) -> String {
    format!("uniq({})", columns.join(","))
}

fn write_result<W: Write>(
    query: &Select,
    output: &mut W,
    columns: &[String],
    count: u64,
) -> Result<(), ExecutionError> {
    write_header(query, output, &header(columns))?;
    if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
        writeln!(output, "{count}")?;
    }
    Ok(())
}

pub(super) fn execute_part<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::ApproxDistinctTuple { columns } = &query.projection else {
        unreachable!("composite distinct requires tuple projection");
    };
    let metadata = inspect_query_source(part)?;
    for column in columns {
        if !metadata.columns.iter().any(|candidate| candidate.name == *column) {
            return Err(ExecutionError::Invalid(format!(
                "unknown composite uniq column: {column}"
            )));
        }
    }
    if query.limit == Some(0) {
        return write_result(query, output, columns, 0);
    }
    let mut names = columns.clone();
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !names.iter().any(|name| name == predicate.column()) {
            names.push(predicate.column().to_owned());
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<String, usize>>();
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<String, String>>();
    let mut predicates = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        predicates.push(typed::compile(branch, &indices, &types)?);
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let mut sketch = HyperLogLog::default();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &predicates) {
                continue;
            }
            // Tuple columns occupy the front of the requested scan layout.
            // NULL in any argument excludes the row, as for nullable uniq().
            sketch.observe_tuple(&batch[..columns.len()], row)?;
        }
    }
    write_result(query, output, columns, sketch.estimate())
}

pub(super) fn execute_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    columns: &[String],
    output: &mut W,
) -> Result<(), ExecutionError> {
    if columns.iter().any(|column| column != "number") {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    if query.limit == Some(0) || query.offset.unwrap_or(0) > 0 {
        return write_result(query, output, columns, 0);
    }
    let conditions = compile_numbers_predicates(query)?;
    let result = if step == 0 {
        u64::from(rows > 0 && number_matches_all(Number::Unsigned(u128::from(start)), &conditions))
    } else if query.predicates.is_empty() && query.or_predicates.is_empty() {
        rows
    } else if let Some(count) = count_monotonic_numbers(&conditions, start, rows, step) {
        count
    } else {
        if rows > 10_000_000 {
            return Err(ExecutionError::Invalid(
                "large generated composite uniq requires monotonic WHERE predicates".into(),
            ));
        }
        let mut count = 0u64;
        for index in 0..rows {
            let value = Number::Unsigned(
                u128::from(start) + u128::from(index) * u128::from(step),
            );
            if number_matches_all(value, &conditions) {
                count += 1;
            }
        }
        count
    };
    write_result(query, output, columns, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_header_is_stable_in_projection_order() {
        assert_eq!(
            header(&["uid".into(), "region".into(), "device".into()]),
            "uniq(uid,region,device)"
        );
    }
}
