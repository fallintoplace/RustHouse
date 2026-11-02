//! Stream numeric aggregates over joined rows without materializing pair output.

use super::super::AggregateState;
use super::*;
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
    let left_column_count = left_names.len();
    let index = resolve(query, column, left_names, right_names)?;
    let metadata = if index < left_column_count {
        inspect_query_source(left_path)?
    } else {
        inspect_query_source(right_path)?
    };
    let source_index = if index < left_column_count {
        index
    } else {
        index - left_column_count
    };
    let data_type = base_type(&metadata.columns[source_index].data_type);
    if matches!(data_type, "String" | "UUID" | "Date" | "DateTime")
        || data_type.starts_with("Array(")
        || data_type.starts_with("Map(")
    {
        return Err(invalid("JOIN numeric aggregate requires a numeric column"));
    }
    let header = query.aggregate_alias.clone().unwrap_or_else(|| {
        format!("{}({}.{})", function.name(), column.relation, column.name)
    });
    if query.limit == Some(0) || query.offset != 0 {
        writeln!(output, "{header}")?;
        return Ok(());
    }

    let selected = [(index, header.clone(), None)];
    let right_meta = inspect_query_source(right_path)?;
    let keep = retained_build_columns(
        right_names,
        right_keys,
        &selected,
        null_filter,
        left_column_count,
    );
    let right_rows = read_projected_build_rows(
        right_path,
        right_names,
        &keep,
        right_meta.rows,
        MAX_STREAM_BUILD_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    // The shared builder reuses a probe key on duplicates and can partition
    // large dimension builds without changing ANY's first-match behavior.
    let hash = build_join_hash(
        &right_rows,
        right_keys,
        query.any,
        super::super::query_workers(4),
    )?;
    let mut state = AggregateState::new(function);
    if !hash.is_empty() {
        probe_inner(
            left_path,
            left_names,
            left_keys,
            &right_rows,
            &hash,
            ProbeOptions {
                materialize_rows: index < left_column_count,
                materialize_after: 0,
                stop_after: Some(usize::MAX),
                any: query.any,
                null_filter,
            },
            |left, right| {
                let cell = if index < left_column_count {
                    &left[index]
                } else {
                    &right[index - left_column_count]
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
