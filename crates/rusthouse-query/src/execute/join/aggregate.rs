//! Stream numeric aggregates over joined rows without materializing pair output.

use super::*;
use super::super::AggregateState;
use crate::AggregateFunction;

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_numeric<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    left_names: &[String],
    right_names: &[String],
    left_keys: &[usize],
    right_keys: &[usize],
    null_filter: Option<(usize, bool)>,
    function: AggregateFunction,
    column: &JoinColumn,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let width = left_names.len();
    let index = resolve(query, column, left_names, right_names)?;
    let from_left = index < width;
    let metadata = if from_left {
        inspect_query_source(left_path)?
    } else {
        inspect_query_source(right_path)?
    };
    let source_index = if from_left { index } else { index - width };
    let data_type = base_type(&metadata.columns[source_index].data_type);
    if matches!(data_type, "String" | "UUID" | "Date" | "DateTime")
        || data_type.starts_with("Array(")
        || data_type.starts_with("Map(")
    {
        return Err(invalid("JOIN numeric aggregate requires a numeric column"));
    }
    let header = format!("{}({}.{})", function.name(), column.relation, column.name);
    if query.limit == Some(0) || query.offset != 0 {
        writeln!(output, "{header}")?;
        return Ok(());
    }

    let selected = [(index, header.clone(), None)];
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
    let mut state = AggregateState::new(function);
    if !hash.is_empty() {
        probe_inner(
            left_path,
            left_names,
            left_keys,
            &right_rows,
            &hash,
            ProbeOptions {
                materialize_rows: from_left,
                materialize_after: 0,
                stop_after: Some(usize::MAX),
                any: query.any,
                null_filter,
            },
            |left, right| {
                let cell = if from_left {
                    &left[index]
                } else {
                    &right[index - width]
                };
                match cell {
                    typed::Cell::Numeric(number) => state.update(*number),
                    typed::Cell::Null => Ok(()),
                    _ => Err(invalid("JOIN numeric aggregate requires a numeric column")),
                }
            },
        )?;
    }
    writeln!(output, "{header}")?;
    writeln!(output, "{}", state.render())?;
    Ok(())
}
