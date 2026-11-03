//! Streaming typed comparisons after unordered two-table INNER joins.
//! Only the small dimension hash is retained; fact rows are decoded in batches.

use super::*;
use rusthouse_storage::TypedQueryReader;

enum FilterTarget {
    Literal(typed::Cell),
    Column(usize),
}
struct BoundFilter {
    column: usize,
    operator: crate::Operator,
    target: FilterTarget,
}

// ALL INNER JOINs may discard dimension rows that fail a dimension-only
// predicate before building the hash table. ANY must keep the first matching
// key even if that row later fails the predicate.
fn prefilter_dimension_rows(
    rows: &mut Vec<Vec<typed::Cell>>,
    filters: &[BoundFilter],
    left_width: usize,
    any: bool,
) {
    if !any {
        rows.retain(|row| {
            filters
                .iter()
                .filter(|filter| filter.column >= left_width)
                .all(|filter| {
                    let target = match &filter.target {
                        FilterTarget::Literal(value) => value,
                        FilterTarget::Column(index) => &row[*index - left_width],
                    };
                    typed::matches_comparison(
                        &row[filter.column - left_width],
                        target,
                        filter.operator,
                    )
                })
        });
    }
}

fn paged_match_limit(offset: usize, limit: Option<usize>) -> Option<u64> {
    limit
        .filter(|&limit| limit <= MAX_RESULT_ROWS)
        .map(|limit| offset.saturating_add(limit) as u64)
}

// Read only JOIN keys, the fact-side filter and visible fact-side output.
// The validation scan needs no output columns at all.
fn fact_probe_columns(
    width: usize,
    keys: &[usize],
    filter_columns: &[usize],
    selected: &[(usize, String, Option<usize>)],
    emit: bool,
) -> Vec<usize> {
    let mut columns = keys.to_vec();
    columns.extend(
        filter_columns
            .iter()
            .copied()
            .filter(|column| *column < width),
    );
    if emit {
        columns.extend(
            selected
                .iter()
                .filter_map(|(index, _, _)| (*index < width).then_some(*index)),
        );
    }
    columns.sort_unstable();
    columns.dedup();
    if columns.is_empty() {
        columns.push(0);
    }
    columns
}

#[allow(clippy::too_many_arguments)]
fn scan<W: Write>(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    hash: &HashMap<Vec<typed::Key>, Bucket>,
    filters: &[BoundFilter],
    selected: &[(usize, String, Option<usize>)],
    any: bool,
    offset: usize,
    limit: Option<usize>,
    emit: bool,
    count_only: bool,
    output: &mut W,
) -> Result<u64, ExecutionError> {
    let width = left_names.len();
    let filter_columns = filters
        .iter()
        .flat_map(|filter| {
            std::iter::once(filter.column).chain(match filter.target {
                FilterTarget::Column(index) => Some(index),
                FilterTarget::Literal(_) => None,
            })
        })
        .collect::<Vec<_>>();
    let columns = fact_probe_columns(width, left_keys, &filter_columns, selected, emit);
    let mut source_to_batch = vec![usize::MAX; width];
    let names = columns
        .iter()
        .enumerate()
        .map(|(batch_index, &source_index)| {
            source_to_batch[source_index] = batch_index;
            left_names[source_index].clone()
        })
        .collect::<Vec<_>>();
    let key_columns = left_keys
        .iter()
        .map(|&index| source_to_batch[index])
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(left_path, &names)?;
    let mut key = Vec::with_capacity(left_keys.len());
    // Cache each distinct left projection separately across all dimension
    // matches for this fact row. One shared Cell corrupts multi-column output.
    let mut left_cells = vec![None; width];
    let mut output_row = if emit {
        Vec::with_capacity(256)
    } else {
        Vec::new()
    };
    let mut matched = 0u64;
    let mut written = 0usize;
    // A bounded page is still bounded when OFFSET + LIMIT overflows usize.
    // Treat the saturated end as an unreachable match count, rather than
    // incorrectly falling into the unpaged 131072-row result cap.
    let max_matches = paged_match_limit(offset, limit);
    while let Some(batch) = reader.next_batch()? {
        for row_index in 0..batch[0].len() {
            // INNER JOIN predicates referencing only the fact side are safe
            // to evaluate before hashing, including for ANY joins.
            if !filters
                .iter()
                .filter(|filter| filter.column < width)
                .all(|filter| {
                    let candidate = typed::cell(&batch[source_to_batch[filter.column]], row_index);
                    match &filter.target {
                        FilterTarget::Literal(value) => {
                            typed::matches_comparison(&candidate, value, filter.operator)
                        }
                        FilterTarget::Column(index) => {
                            let target = typed::cell(&batch[source_to_batch[*index]], row_index);
                            typed::matches_comparison(&candidate, &target, filter.operator)
                        }
                    }
                })
            {
                continue;
            }
            if !batch_key_into(&batch, row_index, &key_columns, &mut key) {
                continue;
            }
            let Some(matches) = hash.get(key.as_slice()) else {
                continue;
            };
            if emit {
                left_cells.fill(None);
            }
            for &right_index in matches.iter().take(if any { 1 } else { usize::MAX }) {
                // Dimension predicates must stay after ANY match selection.
                // Filtering build rows first would change first-match semantics.
                if !filters
                    .iter()
                    .filter(|filter| filter.column >= width)
                    .all(|filter| {
                        let candidate = &right_rows[right_index][filter.column - width];
                        let target = match &filter.target {
                            FilterTarget::Literal(value) => value,
                            FilterTarget::Column(index) => &right_rows[right_index][*index - width],
                        };
                        typed::matches_comparison(candidate, target, filter.operator)
                    })
                {
                    continue;
                }
                matched = matched
                    .checked_add(1)
                    .ok_or_else(|| invalid("JOIN match count overflow"))?;
                if !count_only && max_matches.is_none() && matched > MAX_RESULT_ROWS as u64 {
                    return Err(invalid("JOIN produces more than 131072 rows"));
                }
                if matched <= offset as u64 {
                    continue;
                }
                if emit {
                    output_row.clear();
                    for (position, (index, _, fallback)) in selected.iter().enumerate() {
                        if position > 0 {
                            output_row.push(b'\t');
                        }
                        let value: &typed::Cell = if *index < width {
                            left_cells[*index].get_or_insert_with(|| {
                                typed::cell(&batch[source_to_batch[*index]], row_index)
                            })
                        } else {
                            &right_rows[right_index][*index - width]
                        };
                        let value = if matches!(value, typed::Cell::Null) {
                            fallback
                                .and_then(|index| {
                                    if index >= width {
                                        right_rows[right_index].get(index - width)
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(value)
                        } else {
                            value
                        };
                        typed::write_value(&mut output_row, value)?;
                    }
                    output_row.push(b'\n');
                    output.write_all(&output_row)?;
                }
                written += 1;
                if limit.is_some_and(|limit| written >= limit) {
                    return Ok(matched);
                }
            }
        }
    }
    Ok(matched)
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
    let width = left_names.len();
    let left_meta = inspect_query_source(left_path)?;
    let right_meta = inspect_query_source(right_path)?;
    let mut filters = query
        .where_comparison
        .iter()
        .chain(&query.where_comparisons_extra)
        .map(|(column, operator, text)| {
            let index = resolve(query, column, left_names, right_names)?;
            let metadata = if index < width {
                &left_meta
            } else {
                &right_meta
            };
            let local_index = if index < width { index } else { index - width };
            let literal = typed::parse_literal(&metadata.columns[local_index].data_type, text)?;
            Ok(BoundFilter {
                column: index,
                operator: *operator,
                target: FilterTarget::Literal(literal),
            })
        })
        .collect::<Result<Vec<_>, ExecutionError>>()?;
    for (left, operator, right) in &query.where_column_comparisons {
        let left_index = resolve(query, left, left_names, right_names)?;
        let right_index = resolve(query, right, left_names, right_names)?;
        if (left_index < width) != (right_index < width) {
            return Err(invalid("JOIN column comparisons must use one input table"));
        }
        let (metadata, left_local, right_local) = if left_index < width {
            (&left_meta, left_index, right_index)
        } else {
            (&right_meta, left_index - width, right_index - width)
        };
        let left_type = base_type(&metadata.columns[left_local].data_type);
        let right_type = base_type(&metadata.columns[right_local].data_type);
        let numeric = |kind: &str| {
            matches!(
                kind,
                "UInt8"
                    | "UInt16"
                    | "UInt32"
                    | "UInt64"
                    | "UInt128"
                    | "Int8"
                    | "Int16"
                    | "Int32"
                    | "Int64"
                    | "Int128"
                    | "Float32"
                    | "Float64"
                    | "Date"
                    | "Date32"
                    | "DateTime"
            )
        };
        if left_type != right_type && !(numeric(left_type) && numeric(right_type)) {
            return Err(invalid("JOIN column comparison requires compatible types"));
        }
        filters.push(BoundFilter {
            column: left_index,
            operator: *operator,
            target: FilterTarget::Column(right_index),
        });
    }
    if filters.is_empty() {
        return Err(invalid("JOIN comparison filter missing"));
    }
    let count_only = query.aggregate == Some(JoinAggregate::CountStar);
    let mut header = if count_only {
        query
            .aggregate_alias
            .clone()
            .unwrap_or_else(|| "count".to_owned())
    } else {
        String::new()
    };
    if !count_only {
        for (index, (_, name, _)) in selected.iter().enumerate() {
            if index != 0 {
                header.push('\t');
            }
            header.push_str(name);
        }
    }
    if query.limit == Some(0) || (count_only && query.offset != 0) {
        writeln!(output, "{header}")?;
        return Ok(());
    }

    // Aggregates must not retain unused JOIN projection columns.
    let mut kept = if count_only {
        Vec::new()
    } else {
        selected.to_vec()
    };
    for filter in &filters {
        if filter.column >= width {
            kept.push((filter.column, String::new(), None));
        }
        if let FilterTarget::Column(index) = filter.target {
            if index >= width {
                kept.push((index, String::new(), None));
            }
        }
    }
    let columns = retained_build_columns(right_names, right_keys, &kept, None, width);
    let mut right_rows = read_projected_build_rows(
        right_path,
        right_names,
        &columns,
        right_meta.rows,
        MAX_STREAM_BUILD_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    prefilter_dimension_rows(&mut right_rows, &filters, width, query.any);
    // Preserve the dimension predicate before building. The shared builder
    // retains source order for ALL matches and the first match for ANY.
    let hash = build_join_hash(
        &right_rows,
        right_keys,
        query.any,
        super::super::query_workers(4),
    )?;
    if count_only {
        // Scalar COUNT(*) counts every passing pair without the materialized
        // result-row cap, and LIMIT applies to its single result row.
        let count = if hash.is_empty() {
            0
        } else {
            scan(
                left_path,
                left_names,
                left_keys,
                &right_rows,
                &hash,
                &filters,
                &[],
                query.any,
                0,
                None,
                false,
                true,
                &mut std::io::sink(),
            )?
        };
        writeln!(output, "{header}\n{count}")?;
        return Ok(());
    }
    if !hash.is_empty() {
        // Validate the full stream and output row cap before writing output.
        scan(
            left_path,
            left_names,
            left_keys,
            &right_rows,
            &hash,
            &filters,
            selected,
            query.any,
            query.offset,
            query.limit,
            false,
            false,
            &mut std::io::sink(),
        )?;
    }
    writeln!(output, "{header}")?;
    if hash.is_empty() {
        return Ok(());
    }
    scan(
        left_path,
        left_names,
        left_keys,
        &right_rows,
        &hash,
        &filters,
        selected,
        query.any,
        query.offset,
        query.limit,
        true,
        false,
        output,
    )
    .map(|_| ())
}

#[cfg(test)]
mod prefilter_right_comparison_tests {
    use super::*;

    #[test]
    fn all_join_filters_dimension_before_hash_but_any_keeps_first_key() {
        let mk = |key, value| {
            vec![
                typed::Cell::Numeric(Number::Signed(key)),
                typed::Cell::Numeric(Number::Signed(value)),
            ]
        };
        let rows = vec![mk(3, 5), mk(3, 15), mk(4, 20)];
        let filter = BoundFilter {
            column: 3,
            operator: crate::Operator::Gt,
            target: FilterTarget::Literal(typed::Cell::Numeric(Number::Signed(10))),
        };
        let mut all = rows.clone();
        prefilter_dimension_rows(&mut all, std::slice::from_ref(&filter), 2, false);
        let values = |rows: &[Vec<typed::Cell>]| {
            rows.iter()
                .map(|row| match &row[1] {
                    typed::Cell::Numeric(Number::Signed(value)) => *value,
                    _ => panic!("expected signed test value"),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(values(&all), vec![15, 20]);

        let mut any = rows.clone();
        prefilter_dimension_rows(&mut any, std::slice::from_ref(&filter), 2, true);
        assert_eq!(values(&any), vec![5, 15, 20], "ANY preserves first match");

        let mut fact_filter = rows.clone();
        let left_filter = BoundFilter {
            column: 0,
            ..filter
        };
        prefilter_dimension_rows(
            &mut fact_filter,
            std::slice::from_ref(&left_filter),
            2,
            false,
        );
        assert_eq!(values(&fact_filter), vec![5, 15, 20]);
    }
}

#[cfg(test)]
mod paged_join_match_limit_tests {
    use super::*;

    #[test]
    fn overflowing_offset_does_not_turn_a_bounded_page_into_an_unpaged_join() {
        assert_eq!(paged_match_limit(0, Some(1)), Some(1));
        assert_eq!(paged_match_limit(10, Some(20)), Some(30));
        assert_eq!(
            paged_match_limit(usize::MAX, Some(1)),
            Some(usize::MAX as u64)
        );
        assert_eq!(
            paged_match_limit(usize::MAX - 1, Some(20)),
            Some(usize::MAX as u64)
        );
        assert_eq!(paged_match_limit(0, Some(MAX_RESULT_ROWS + 1)), None);
        assert_eq!(paged_match_limit(0, None), None);
    }
}

#[cfg(test)]
mod fact_probe_column_tests {
    use super::*;

    #[test]
    fn narrow_validation_and_output_read_only_required_columns() {
        let projected = vec![
            (5, String::new(), None),
            (8, String::new(), None),
            (5, String::new(), None),
        ];
        assert_eq!(
            fact_probe_columns(6, &[1, 3], &[4], &projected, false),
            vec![1, 3, 4]
        );
        assert_eq!(
            fact_probe_columns(6, &[1, 3], &[4], &projected, true),
            vec![1, 3, 4, 5]
        );
        assert_eq!(
            fact_probe_columns(6, &[1], &[8], &projected, false),
            vec![1]
        );
        assert_eq!(fact_probe_columns(6, &[], &[8], &projected, false), vec![0]);
    }
}
