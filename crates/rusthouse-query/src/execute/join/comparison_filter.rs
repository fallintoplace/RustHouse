//! Streaming typed comparisons after unordered two-table INNER joins.
//! Only the small dimension hash is retained; fact rows are decoded in batches.

use super::*;
use rusthouse_storage::TypedQueryReader;

enum FilterTarget {
    Literal(typed::Cell),
    Column(usize),
    Values(Vec<typed::Cell>),
    Pattern(typed::TextPattern, bool, bool),
}
struct BoundFilter {
    column: usize,
    operator: crate::Operator,
    target: FilterTarget,
}

fn matches_pattern(
    candidate: &typed::Cell,
    pattern: &typed::TextPattern,
    insensitive: bool,
    negated: bool,
) -> bool {
    match candidate {
        typed::Cell::Text(bytes) => {
            typed::text_pattern_matches(bytes, pattern, insensitive) != negated
        }
        _ => false,
    }
}

fn is_cross_input(filter: &BoundFilter, width: usize) -> bool {
    matches!(filter.target, FilterTarget::Column(index) if (filter.column < width) != (index < width))
}

struct CaseMetric {
    tests: Vec<(crate::Operator, typed::Cell)>,
    all: bool,
}

struct CaseGroups {
    metrics: Vec<CaseMetric>,
    groups: HashMap<typed::Key, (typed::Cell, Vec<u64>)>,
    bytes: usize,
}
impl CaseGroups {
    fn add(&mut self, values: &[typed::Cell]) -> Result<(), ExecutionError> {
        let key = values[0].key();
        if !self.groups.contains_key(&key) {
            if self.groups.len() >= 100_000 {
                return Err(invalid("too many JOIN groups"));
            }
            let variable = match &values[0] {
                typed::Cell::Text(bytes) => bytes.len().saturating_mul(2),
                typed::Cell::Uuid(value) => value.len().saturating_mul(2),
                _ => 0,
            };
            let size = std::mem::size_of::<(typed::Key, typed::Cell, Vec<u64>)>()
                .saturating_add(
                    self.metrics
                        .len()
                        .saturating_mul(std::mem::size_of::<u64>()),
                )
                .saturating_add(variable);
            self.bytes = self
                .bytes
                .checked_add(size)
                .ok_or_else(|| invalid("JOIN group memory overflow"))?;
            if self.bytes > 64 * 1024 * 1024 {
                return Err(invalid("JOIN grouped hash memory budget exceeded"));
            }
            self.groups.insert(
                key.clone(),
                (values[0].clone(), vec![0; self.metrics.len()]),
            );
        }
        let counts = &mut self.groups.get_mut(&key).expect("group inserted").1;
        for (i, metric) in self.metrics.iter().enumerate() {
            let matches = |(operator, literal): &(crate::Operator, typed::Cell)| {
                typed::matches_comparison(&values[i + 1], literal, *operator)
            };
            let matches = if metric.all {
                metric.tests.iter().all(matches)
            } else {
                metric.tests.iter().any(matches)
            };
            if matches {
                counts[i] = counts[i]
                    .checked_add(1)
                    .ok_or_else(|| invalid("JOIN grouped count overflow"))?;
            }
        }
        Ok(())
    }
}

fn write_case_header<W: Write>(
    query: &JoinSelect,
    selected: &[(usize, String, Option<usize>)],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let name = &selected
        .first()
        .ok_or_else(|| invalid("JOIN group key missing"))?
        .1;
    write!(output, "{name}")?;
    for (_, _, alias) in &query.grouped_case_counts {
        write!(output, "\t{alias}")?;
    }
    writeln!(output)?;
    Ok(())
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
                .filter(|filter| filter.column >= left_width && !is_cross_input(filter, left_width))
                .all(|filter| {
                    let candidate = &row[filter.column - left_width];
                    match &filter.target {
                        FilterTarget::Literal(value) => {
                            typed::matches_comparison(candidate, value, filter.operator)
                        }
                        FilterTarget::Column(index) => typed::matches_comparison(
                            candidate,
                            &row[*index - left_width],
                            filter.operator,
                        ),
                        FilterTarget::Values(values) => values.iter().any(|value| {
                            typed::matches_comparison(candidate, value, crate::Operator::Eq)
                        }),
                        FilterTarget::Pattern(pattern, insensitive, negated) => {
                            matches_pattern(candidate, pattern, *insensitive, *negated)
                        }
                    }
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
fn scan<H: JoinLookup, W: Write>(
    left_path: &Path,
    left_names: &[String],
    left_keys: &[usize],
    right_rows: &[Vec<typed::Cell>],
    hash: &H,
    filters: &[BoundFilter],
    selected: &[(usize, String, Option<usize>)],
    any: bool,
    offset: usize,
    limit: Option<usize>,
    emit: bool,
    count_only: bool,
    mut grouped: Option<&mut CaseGroups>,
    output: &mut W,
) -> Result<u64, ExecutionError> {
    let width = left_names.len();
    let filter_columns = filters
        .iter()
        .flat_map(|filter| {
            std::iter::once(filter.column).chain(match filter.target {
                FilterTarget::Column(index) => Some(index),
                FilterTarget::Literal(_) | FilterTarget::Values(_) | FilterTarget::Pattern(..) => {
                    None
                }
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
                .filter(|filter| filter.column < width && !is_cross_input(filter, width))
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
                        FilterTarget::Values(values) => values.iter().any(|value| {
                            typed::matches_comparison(&candidate, value, crate::Operator::Eq)
                        }),
                        FilterTarget::Pattern(pattern, insensitive, negated) => {
                            matches_pattern(&candidate, pattern, *insensitive, *negated)
                        }
                    }
                })
            {
                continue;
            }
            if !batch_key_into(&batch, row_index, &key_columns, &mut key) {
                continue;
            }
            let Some(matches) = hash.lookup(key.as_slice()) else {
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
                    .filter(|filter| filter.column >= width && !is_cross_input(filter, width))
                    .all(|filter| {
                        let candidate = &right_rows[right_index][filter.column - width];
                        match &filter.target {
                            FilterTarget::Literal(value) => {
                                typed::matches_comparison(candidate, value, filter.operator)
                            }
                            FilterTarget::Column(index) => typed::matches_comparison(
                                candidate,
                                &right_rows[right_index][*index - width],
                                filter.operator,
                            ),
                            FilterTarget::Values(values) => values.iter().any(|value| {
                                typed::matches_comparison(candidate, value, crate::Operator::Eq)
                            }),
                            FilterTarget::Pattern(pattern, insensitive, negated) => {
                                matches_pattern(candidate, pattern, *insensitive, *negated)
                            }
                        }
                    })
                {
                    continue;
                }
                if !filters
                    .iter()
                    .filter(|filter| is_cross_input(filter, width))
                    .all(|filter| {
                        let FilterTarget::Column(other) = filter.target else {
                            unreachable!()
                        };
                        let value = |index: usize| {
                            if index < width {
                                typed::cell(&batch[source_to_batch[index]], row_index)
                            } else {
                                right_rows[right_index][index - width].clone()
                            }
                        };
                        typed::matches_comparison(
                            &value(filter.column),
                            &value(other),
                            filter.operator,
                        )
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
                if let Some(groups) = grouped.as_deref_mut() {
                    let values = selected
                        .iter()
                        .map(|(index, _, _)| {
                            if *index < width {
                                typed::cell(&batch[source_to_batch[*index]], row_index)
                            } else {
                                right_rows[right_index][*index - width].clone()
                            }
                        })
                        .collect::<Vec<_>>();
                    groups.add(&values)?;
                } else if emit {
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
        let column_type = |index: usize| {
            if index < width {
                base_type(&left_meta.columns[index].data_type)
            } else {
                base_type(&right_meta.columns[index - width].data_type)
            }
        };
        let left_type = column_type(left_index);
        let right_type = column_type(right_index);
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
    for (column, texts) in &query.where_in_filters {
        let index = resolve(query, column, left_names, right_names)?;
        let metadata = if index < width {
            &left_meta
        } else {
            &right_meta
        };
        let local_index = if index < width { index } else { index - width };
        let values = texts
            .iter()
            .map(|text| typed::parse_literal(&metadata.columns[local_index].data_type, text))
            .collect::<Result<Vec<_>, _>>()?;
        filters.push(BoundFilter {
            column: index,
            operator: crate::Operator::Eq,
            target: FilterTarget::Values(values),
        });
    }
    for (column, text, insensitive, negated) in &query.where_like_filters {
        let index = resolve(query, column, left_names, right_names)?;
        let metadata = if index < width {
            &left_meta
        } else {
            &right_meta
        };
        let local = if index < width { index } else { index - width };
        if base_type(&metadata.columns[local].data_type) != "String" {
            return Err(invalid("JOIN LIKE requires a String column"));
        }
        let pattern = typed::compile_text_pattern(text)?;
        filters.push(BoundFilter {
            column: index,
            operator: crate::Operator::Eq,
            target: FilterTarget::Pattern(pattern, *insensitive, *negated),
        });
    }
    if filters.is_empty() && query.grouped_case_counts.is_empty() {
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
    if query.grouped_case_counts.is_empty()
        && (query.limit == Some(0) || (count_only && query.offset != 0))
    {
        writeln!(output, "{header}")?;
        return Ok(());
    }

    let mut case_selected = selected.to_vec();
    let mut case_metrics = Vec::new();
    for (filter, value, _) in &query.grouped_case_counts {
        if !matches!(value, crate::GroupedCaseValue::Count) {
            return Err(invalid(
                "conditional numeric SUM requires a streaming INNER join chain",
            ));
        }
        let (column, comparisons, all) = match filter {
            crate::GroupedFilter::CaseElseZero {
                column,
                operator,
                literal,
            } => (column, vec![(*operator, literal.clone())], true),
            crate::GroupedFilter::CaseElseZeroCombined {
                column,
                comparisons,
                all,
            } => (column, comparisons.clone(), *all),
            crate::GroupedFilter::CaseExpression { .. } | crate::GroupedFilter::CaseNull { .. } => {
                return Err(invalid(
                    "expression JOIN CASE requires a streaming INNER join chain",
                ));
            }
            _ => return Err(invalid("unsupported JOIN CASE metric")),
        };
        let index = resolve(
            query,
            &crate::JoinColumn {
                relation: String::new(),
                name: column.clone(),
            },
            left_names,
            right_names,
        )?;
        let metadata = if index < width {
            &left_meta
        } else {
            &right_meta
        };
        let local = if index < width { index } else { index - width };
        let tests = comparisons
            .into_iter()
            .map(|(operator, literal)| {
                Ok((
                    operator,
                    typed::parse_literal(&metadata.columns[local].data_type, &literal)?,
                ))
            })
            .collect::<Result<Vec<_>, ExecutionError>>()?;
        case_selected.push((index, String::new(), None));
        case_metrics.push(CaseMetric { tests, all });
    }
    if !query.grouped_case_counts.is_empty() && query.limit == Some(0) {
        return write_case_header(query, selected, output);
    }
    // Aggregates must not retain unused JOIN projection columns.
    let mut kept = if count_only {
        Vec::new()
    } else {
        case_selected.clone()
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
    if !query.grouped_case_counts.is_empty() {
        let group_index = selected
            .first()
            .ok_or_else(|| invalid("JOIN group key missing"))?
            .0;
        let group_type = if group_index < width {
            &left_meta.columns[group_index].data_type
        } else {
            &right_meta.columns[group_index - width].data_type
        };
        let mut groups = CaseGroups {
            metrics: case_metrics,
            groups: HashMap::new(),
            bytes: 0,
        };
        if query.limit != Some(0) && !hash.is_empty() {
            scan(
                left_path,
                left_names,
                left_keys,
                &right_rows,
                &hash,
                &filters,
                &case_selected,
                query.any,
                0,
                None,
                true,
                true,
                Some(&mut groups),
                &mut std::io::sink(),
            )?;
        }
        write_case_header(query, selected, output)?;
        let mut rows = groups.groups.into_values().collect::<Vec<_>>();
        if let Some((_, descending, nulls_first)) = query.order_by.first() {
            rows.sort_by(|a, b| typed::compare_order_cells(&a.0, &b.0, *descending, *nulls_first));
        }
        for (key, counts) in rows
            .iter()
            .skip(query.offset)
            .take(query.limit.unwrap_or(usize::MAX))
        {
            typed::write_typed_value(output, key, group_type)?;
            for count in counts {
                write!(output, "\t{count}")?;
            }
            writeln!(output)?;
        }
        return Ok(());
    }
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
                None,
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
            None,
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
        None,
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

#[cfg(test)]
mod grouped_case_budget_tests {
    use super::*;

    fn groups() -> CaseGroups {
        CaseGroups {
            metrics: vec![CaseMetric {
                tests: vec![(
                    crate::Operator::Eq,
                    typed::Cell::Numeric(Number::Unsigned(1)),
                )],
                all: true,
            }],
            groups: HashMap::new(),
            bytes: 0,
        }
    }

    #[test]
    fn grouped_case_enforces_group_and_byte_limits() {
        let mut groups = groups();
        for value in 0..100_000u128 {
            groups
                .add(&[
                    typed::Cell::Numeric(Number::Unsigned(value)),
                    typed::Cell::Numeric(Number::Unsigned(1)),
                ])
                .unwrap();
        }
        assert!(
            groups
                .add(&[
                    typed::Cell::Numeric(Number::Unsigned(100_000)),
                    typed::Cell::Numeric(Number::Unsigned(1))
                ])
                .is_err()
        );
        assert_eq!(groups.groups.len(), 100_000);
        let mut byte_limited = self::groups();
        byte_limited.bytes = 64 * 1024 * 1024;
        assert!(
            byte_limited
                .add(&[
                    typed::Cell::Numeric(Number::Unsigned(1)),
                    typed::Cell::Numeric(Number::Unsigned(1))
                ])
                .is_err()
        );
        assert!(byte_limited.groups.is_empty());
    }

    #[test]
    fn grouped_case_rejects_count_overflow() {
        let mut groups = groups();
        let key = typed::Cell::Numeric(Number::Unsigned(1));
        groups
            .groups
            .insert(key.key(), (key.clone(), vec![u64::MAX]));
        assert!(
            groups
                .add(&[key, typed::Cell::Numeric(Number::Unsigned(1))])
                .is_err()
        );
    }
}
