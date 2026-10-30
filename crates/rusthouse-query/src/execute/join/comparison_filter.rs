//! Streaming typed comparisons after unordered two-table INNER joins.
//! Only the small dimension hash is retained; fact rows are decoded in batches.

use super::*;
use rusthouse_storage::TypedQueryReader;

struct BoundFilter {
    column: usize,
    operator: crate::Operator,
    literal: typed::Cell,
}

#[allow(clippy::too_many_arguments)]
fn scan<W: Write>(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    hash: &HashMap<Vec<typed::Key>, Bucket>,
    filter: &BoundFilter,
    selected: &[(usize, String, Option<usize>)],
    any: bool,
    offset: usize,
    limit: Option<usize>,
    emit: bool,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let width = left_names.len();
    let mut reader = TypedQueryReader::open(left_path, left_names)?;
    let mut key = Vec::with_capacity(left_keys.len());
    let mut matched = 0u64;
    let mut written = 0usize;
    let max_matches = match limit {
        Some(limit) if limit <= MAX_RESULT_ROWS => {
            offset.checked_add(limit).map(|end| end as u64)
        }
        _ => None,
    };
    while let Some(batch) = reader.next_batch()? {
        for row_index in 0..batch[0].len() {
            // INNER JOIN predicates referencing only the fact side are safe
            // to evaluate before hashing, including for ANY joins.
            if filter.column < width {
                let candidate = typed::cell(&batch[filter.column], row_index);
                if !typed::matches_comparison(
                    &candidate,
                    &filter.literal,
                    filter.operator,
                ) {
                    continue;
                }
            }
            if !batch_key_into(&batch, row_index, left_keys, &mut key) {
                continue;
            }
            let Some(matches) = hash.get(key.as_slice()) else {
                continue;
            };
            for &right_index in matches.iter().take(if any { 1 } else { usize::MAX }) {
                // Dimension predicates must stay after ANY match selection.
                // Filtering build rows first would change first-match semantics.
                if filter.column >= width {
                    let candidate = &right_rows[right_index][filter.column - width];
                    if !typed::matches_comparison(
                        candidate,
                        &filter.literal,
                        filter.operator,
                    ) {
                        continue;
                    }
                }
                matched = matched
                    .checked_add(1)
                    .ok_or_else(|| invalid("JOIN match count overflow"))?;
                if max_matches.is_none() && matched > MAX_RESULT_ROWS as u64 {
                    return Err(invalid("JOIN produces more than 131072 rows"));
                }
                if matched <= offset as u64 {
                    continue;
                }
                if emit {
                    for (position, (index, _, fallback)) in selected.iter().enumerate() {
                        if position > 0 {
                            output.write_all(b"\t")?;
                        }
                        let mut left_cell = None;
                        let value: &typed::Cell = if *index < width {
                            left_cell.get_or_insert_with(|| typed::cell(&batch[*index], row_index))
                        } else {
                            &right_rows[right_index][*index - width]
                        };
                        let value = if matches!(value, typed::Cell::Null) {
                            fallback.and_then(|index| {
                                if index >= width {
                                    right_rows[right_index].get(index - width)
                                } else {
                                    None
                                }
                            }).unwrap_or(value)
                        } else {
                            value
                        };
                        typed::write_value(output, value)?;
                    }
                    output.write_all(b"\n")?;
                }
                written += 1;
                if limit.is_some_and(|limit| written >= limit) {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute<W: Write>(
    query: &JoinSelect,
    left_path: &Path,
    right_path: &Path,
    left_names: &[String],
    right_names: &[String],
    selected: &[(usize, String, Option<usize>)],
    left_keys: &[usize],
    right_keys: &[usize],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let (column, operator, text) = query
        .where_comparison
        .as_ref()
        .ok_or_else(|| invalid("JOIN comparison filter missing"))?;
    let width = left_names.len();
    let index = resolve(query, column, left_names, right_names)?;
    let metadata = if index < width {
        inspect_query_source(left_path)?
    } else {
        inspect_query_source(right_path)?
    };
    let local_index = if index < width { index } else { index - width };
    let literal = typed::parse_literal(&metadata.columns[local_index].data_type, text)?;
    let filter = BoundFilter {
        column: index,
        operator: *operator,
        literal,
    };
    let header = selected
        .iter()
        .map(|(_, name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join("\t");
    if query.limit == Some(0) {
        writeln!(output, "{header}")?;
        return Ok(());
    }

    let right_meta = inspect_query_source(right_path)?;
    let mut kept = selected.to_vec();
    if index >= width {
        kept.push((index, String::new(), None));
    }
    let columns = retained_build_columns(right_names, right_keys, &kept, None, width);
    let right_rows = read_projected_build_rows(
        right_path,
        right_names,
        &columns,
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
    if !hash.is_empty() {
        // Validate the full stream and output row cap before writing output.
        scan(
            left_path, left_names, left_keys, &right_rows, &hash, &filter,
            selected, query.any, query.offset, query.limit, false, &mut std::io::sink(),
        )?;
    }
    writeln!(output, "{header}")?;
    if hash.is_empty() {
        return Ok(());
    }
    scan(
        left_path, left_names, left_keys, &right_rows, &hash, &filter,
        selected, query.any, query.offset, query.limit, true, output,
    )
}
