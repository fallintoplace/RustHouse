use super::ExecutionError;
use super::typed;
use crate::Select;
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let columns = metadata
        .columns
        .iter()
        .map(|column| (column.name.as_str(), column.data_type.as_str()))
        .collect::<HashMap<_, _>>();
    let kind = columns
        .get(column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown column: {column}")))?;
    if !matches!(*kind, "String" | "Nullable(String)") {
        return Err(ExecutionError::Invalid(
            "LENGTH requires a String column".into(),
        ));
    }
    let mut names = vec![column.to_owned()];
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let predicate_column = predicate.column();
        if !names.iter().any(|name| name == predicate_column) {
            names.push(predicate_column.to_owned());
        }
    }
    let indices: HashMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let types: HashMap<_, _> = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }

    writeln!(
        output,
        "{}",
        query
            .alias
            .as_deref()
            .unwrap_or(&format!("length({column})"))
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let mut reader = TypedQueryReader::open(part, &names)?;
    let mut skipped = 0usize;
    let mut written = 0usize;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(usize::MAX);
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::passes(&batch, row, &conditions) {
                continue;
            }
            if skipped < offset {
                skipped += 1;
                continue;
            }
            match &batch[indices[column]] {
                TypedColumn::Text(values) => {
                    writeln!(output, "{}", values.get(row).unwrap().len())?;
                }
                TypedColumn::NullableText { nulls, .. } if nulls[row] != 0 => {
                    writeln!(output, "\\N")?;
                }
                TypedColumn::NullableText { values, .. } => {
                    writeln!(output, "{}", values.get(row).unwrap().len())?;
                }
                _ => unreachable!("LENGTH requires a String column"),
            }
            written += 1;
            if written == limit {
                return Ok(());
            }
        }
    }
    Ok(())
}
