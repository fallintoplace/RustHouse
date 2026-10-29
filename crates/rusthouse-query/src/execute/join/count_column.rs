//! Stream COUNT(nullable_column) over INNER joins without retaining matches.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn execute<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    left_names: &[String],
    right_names: &[String],
    left_keys: &[usize],
    right_keys: &[usize],
    null_filter: Option<(usize, bool)>,
    column: &JoinColumn,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let width = left_names.len();
    let index = resolve(query, column, left_names, right_names)?;
    let header = format!("count({}.{})", column.relation, column.name);
    if query.limit == Some(0) || query.offset > 0 {
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
    let mut count = 0u64;
    if !hash.is_empty() {
        probe_inner(
            left_path,
            left_names,
            left_keys,
            &right_rows,
            &hash,
            ProbeOptions {
                materialize_rows: index < width,
                materialize_after: 0,
                stop_after: Some(usize::MAX),
                any: query.any,
                null_filter,
            },
            |left, right| {
                let cell = if index < width { &left[index] } else { &right[index - width] };
                if !matches!(cell, typed::Cell::Null) {
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| invalid("JOIN COUNT column overflow"))?;
                }
                Ok(())
            },
        )?;
    }
    writeln!(output, "{header}\n{count}")?;
    Ok(())
}
