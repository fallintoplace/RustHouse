use super::ExecutionError;
use super::typed;
use crate::Select;
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

fn checked_text_length_at_row(
    column: &TypedColumn,
    row: usize,
) -> Result<Option<usize>, ExecutionError> {
    let missing = || ExecutionError::Invalid("missing LENGTH string value in batch".into());
    let values = match column {
        TypedColumn::Text(values) => values,
        TypedColumn::NullableText { values, nulls } => {
            if *nulls.get(row).ok_or_else(missing)? != 0 {
                return Ok(None);
            }
            values
        }
        _ => {
            return Err(ExecutionError::Invalid(
                "LENGTH read unexpected column type".into(),
            ));
        }
    };
    values.byte_length(row).map(Some).ok_or_else(missing)
}

fn projection_columns(query: &Select, column: &str) -> Vec<String> {
    let mut names = vec![column.to_owned()];
    let mut seen = HashSet::from([column]);
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let predicate_column = predicate.column();
        if seen.insert(predicate_column) {
            names.push(predicate_column.to_owned());
        }
    }
    names
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let kind = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown column: {column}")))?;
    if !matches!(kind.data_type.as_str(), "String" | "Nullable(String)") {
        return Err(ExecutionError::Invalid(
            "LENGTH requires a String column".into(),
        ));
    }
    let names = projection_columns(query, column);
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

    if let Some(alias) = query.alias.as_deref() {
        writeln!(output, "{alias}")?;
    } else {
        writeln!(output, "length({column})")?;
    }
    if query.limit == Some(0) {
        return Ok(());
    }

    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let mut skipped = 0usize;
    let mut written = 0usize;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(usize::MAX);
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    // Reuse typed mark skipping: LENGTH only needs rows after OFFSET when
    // neither filtering, sorting nor deduplication changes row positions.
    if no_filters && query.order_by.is_none() && query.order_by_extra.is_empty() && !query.distinct
    {
        skipped = reader.skip_prefix_rows(offset)?;
    }
    while let Some(batch) = reader.next_batch()? {
        let rows = batch
            .first()
            .ok_or_else(|| ExecutionError::Invalid("empty LENGTH input batch".into()))?
            .len();
        let values = batch
            .get(indices[column])
            .ok_or_else(|| ExecutionError::Invalid("missing LENGTH input column".into()))?;
        for row in 0..rows {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            if skipped < offset {
                skipped += 1;
                continue;
            }
            match checked_text_length_at_row(values, row)? {
                Some(length) => writeln!(output, "{length}")?,
                None => writeln!(output, "\\N")?,
            }
            written += 1;
            if written == limit {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusthouse_storage::read_legacy_strings;

    #[test]
    fn checks_nullable_and_empty_string_batches() {
        let mut source = b"\x03abc\x00".as_slice();
        let values = read_legacy_strings(&mut source, 2).unwrap();
        let column = TypedColumn::Text(values);
        assert_eq!(checked_text_length_at_row(&column, 0).unwrap(), Some(3));
        assert_eq!(checked_text_length_at_row(&column, 1).unwrap(), Some(0));
        assert!(checked_text_length_at_row(&column, 2).is_err());

        let mut source = b"\x01x\x01y".as_slice();
        let values = read_legacy_strings(&mut source, 2).unwrap();
        let column = TypedColumn::NullableText {
            values,
            nulls: vec![0, 1],
        };
        assert_eq!(checked_text_length_at_row(&column, 0).unwrap(), Some(1));
        assert_eq!(checked_text_length_at_row(&column, 1).unwrap(), None);
        assert!(checked_text_length_at_row(&column, 2).is_err());
    }

    #[test]
    fn rejects_unexpected_column_types() {
        assert!(checked_text_length_at_row(&TypedColumn::Uuid(Vec::new()), 0).is_err());
    }
}
