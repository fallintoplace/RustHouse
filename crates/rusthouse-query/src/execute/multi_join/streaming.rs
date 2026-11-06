//! Streaming, bounded-build execution for INNER join chains.
//! The fact relation is read in batches; only dimension-side build rows persist.

use super::*;
use rusthouse_storage::TypedQueryReader;
use std::collections::HashSet;

use super::super::join::{
    Bucket as JoinBucket, JoinHash, read_compact_projected_build_rows,
    read_compact_projected_build_rows_matching, read_filtered_compact_projected_build_rows,
    single_numeric_join_key, unsigned_composite_join_key,
};

const MAX_STREAM_GROUPS: usize = 100_000;
const MAX_STREAM_GROUP_BYTES: usize = 64 * 1024 * 1024;

// Account for hash entries and owned key payloads alongside the retained rows.
fn hash_key_bytes(key: &[typed::Key], single_numeric: bool) -> Result<usize, ExecutionError> {
    let base = if key.len() == 1 {
        let entry = if single_numeric {
            std::mem::size_of::<(super::super::NumberKey, JoinBucket)>()
        } else {
            std::mem::size_of::<(typed::Key, JoinBucket)>()
        };
        entry.checked_add(96)
    } else if matches!(
        key,
        [
            typed::Key::Numeric(super::super::NumberKey::Unsigned(_)),
            typed::Key::Numeric(super::super::NumberKey::Unsigned(_))
        ]
    ) {
        std::mem::size_of::<((u128, u128), JoinBucket)>().checked_add(96)
    } else {
        std::mem::size_of::<Vec<typed::Key>>()
            .checked_add(96)
            .and_then(|bytes| {
                key.len()
                    .checked_mul(std::mem::size_of::<typed::Key>())
                    .and_then(|slots| bytes.checked_add(slots))
            })
    }
    .ok_or_else(|| invalid("JOIN hash key memory overflow"))?;
    key.iter().try_fold(base, |total, part| {
        let bytes = match part {
            typed::Key::Text(value) => value.len(),
            typed::Key::Uuid(value) => value.len(),
            _ => 0,
        };
        total
            .checked_add(bytes)
            .ok_or_else(|| invalid("JOIN hash key memory overflow"))
    })
}

fn charge_hash_build(retained: &mut usize, bytes: usize) -> Result<(), ExecutionError> {
    *retained = retained
        .checked_add(bytes)
        .filter(|&total| total <= MAX_CHAIN_BYTES)
        .ok_or_else(|| invalid("JOIN hash-build memory budget exceeded"))?;
    Ok(())
}

struct Stage {
    rows: Vec<Vec<typed::Cell>>,
    hash: JoinHash,
    left_keys: Vec<usize>,
    projected_columns: Vec<usize>,
    full_width: usize,
    kind: JoinType,
    any: bool,
}

type GroupRow = (
    Vec<typed::Cell>,
    u64,
    Option<super::super::Number>,
    Option<HashSet<typed::Key>>,
    Option<Box<[Option<super::super::Number>]>>,
    Option<Box<[(Option<super::super::Number>, u64)]>>,
    Option<Box<[Option<super::super::Number>]>>,
    Option<super::super::Number>,
);

#[derive(Clone, Copy)]
enum GroupMetricOrder {
    Case(usize),
    Average(usize),
    Grouping(usize),
    WindowRatio,
}

#[derive(Clone, Copy)]
struct BoundScalarAggregate {
    function: crate::AggregateFunction,
    column: Option<usize>,
    distinct: bool,
}

enum BoundCaseFilter {
    Expression {
        expression: super::super::multi_aggregate::BoundExpr,
        indices: Vec<usize>,
    },
    Comparison {
        index: usize,
        operator: crate::Operator,
        literal: typed::Cell,
    },
    Null {
        index: usize,
        negated: bool,
    },
    Combined {
        index: usize,
        comparisons: Vec<(crate::Operator, typed::Cell)>,
        all: bool,
    },
}

struct BoundCaseMetric {
    filter: BoundCaseFilter,
    value: Option<crate::execute::multi_aggregate::BoundExpr>,
    value_indices: Vec<usize>,
    else_zero: bool,
}

impl BoundCaseMetric {
    fn referenced_indices(&self) -> impl Iterator<Item = &usize> {
        self.filter
            .referenced_indices()
            .iter()
            .chain(self.value_indices.iter())
    }

    fn value_for_row(
        &self,
        matches: bool,
        row: &[typed::Cell],
    ) -> Result<Option<super::super::Number>, ExecutionError> {
        if !matches {
            return Ok(self.else_zero.then_some(super::super::Number::Unsigned(0)));
        }
        let Some(expression) = &self.value else {
            return Ok(Some(super::super::Number::Unsigned(1)));
        };
        crate::execute::multi_aggregate::eval_row_arithmetic(expression, row)
    }
}

impl BoundCaseFilter {
    fn referenced_indices(&self) -> &[usize] {
        match self {
            Self::Expression { indices, .. } => indices,
            Self::Comparison { index, .. }
            | Self::Null { index, .. }
            | Self::Combined { index, .. } => std::slice::from_ref(index),
        }
    }

    fn passes(&self, row: &[typed::Cell]) -> Result<bool, ExecutionError> {
        match self {
            Self::Expression { expression, .. } => {
                let value = super::super::multi_aggregate::eval_row_arithmetic(expression, row)?;
                Ok(match value {
                    Some(super::super::Number::Unsigned(value)) => value != 0,
                    Some(super::super::Number::Signed(value)) => value != 0,
                    Some(super::super::Number::Float(value)) => value != 0.0,
                    None => false,
                })
            }
            Self::Comparison {
                index,
                operator,
                literal,
            } => Ok(typed::matches_comparison(&row[*index], literal, *operator)),
            Self::Null { index, negated } => {
                Ok(matches!(&row[*index], typed::Cell::Null) != *negated)
            }
            Self::Combined {
                index,
                comparisons,
                all,
            } => Ok(if *all {
                comparisons.iter().all(|(operator, literal)| {
                    typed::matches_comparison(&row[*index], literal, *operator)
                })
            } else {
                comparisons.iter().any(|(operator, literal)| {
                    typed::matches_comparison(&row[*index], literal, *operator)
                })
            }),
        }
    }
}

fn bind_case_filter(
    filter: &crate::GroupedFilter,
    relations: &[Relation],
) -> Result<BoundCaseFilter, ExecutionError> {
    let resolve = |name: &str| {
        let (relation, name) = name.split_once('\0').unwrap_or(("", name));
        super::column_index(
            relations,
            &crate::JoinColumn {
                relation: relation.to_owned(),
                name: name.to_owned(),
            },
        )
    };
    match filter {
        crate::GroupedFilter::CaseExpression { condition, .. } => {
            let mut names = Vec::new();
            condition.referenced_columns(&mut names);
            let mut indices = HashMap::with_capacity(names.len());
            let mut referenced = Vec::with_capacity(names.len());
            for name in names {
                let index = resolve(&name)?;
                indices.insert(name, index);
                referenced.push(index);
            }
            Ok(BoundCaseFilter::Expression {
                expression: super::super::multi_aggregate::BoundExpr::bind(condition, &indices)?,
                indices: referenced,
            })
        }
        crate::GroupedFilter::CaseElseZero {
            column,
            operator,
            literal,
        }
        | crate::GroupedFilter::Comparison {
            column,
            operator,
            literal,
        } => {
            let index = resolve(column)?;
            let literal = typed::parse_literal(super::column_type(relations, index), literal)?;
            Ok(BoundCaseFilter::Comparison {
                index,
                operator: *operator,
                literal,
            })
        }
        crate::GroupedFilter::CaseNull { column, negated } => Ok(BoundCaseFilter::Null {
            index: resolve(column)?,
            negated: *negated,
        }),
        crate::GroupedFilter::CaseElseZeroCombined {
            column,
            comparisons,
            all,
        } => {
            let index = resolve(column)?;
            let data_type = super::column_type(relations, index);
            let comparisons = comparisons
                .iter()
                .map(|(operator, literal)| {
                    Ok((*operator, typed::parse_literal(data_type, literal)?))
                })
                .collect::<Result<Vec<_>, ExecutionError>>()?;
            Ok(BoundCaseFilter::Combined {
                index,
                comparisons,
                all: *all,
            })
        }
        crate::GroupedFilter::Truthy(_) => Err(invalid(
            "grouped CASE count requires an explicit comparison",
        )),
    }
}

fn bind_case_metric(
    filter: &crate::GroupedFilter,
    case_value: &crate::GroupedCaseValue,
    relations: &[Relation],
) -> Result<BoundCaseMetric, ExecutionError> {
    let mut expression_indices = HashMap::new();
    let mut unqualified = HashMap::<String, Option<usize>>::new();
    for relation in relations {
        for (offset, name) in relation.names.iter().enumerate() {
            let index = relation.start + offset;
            expression_indices.insert(format!("{}\0{name}", relation.alias), index);
            unqualified
                .entry(name.clone())
                .and_modify(|index| *index = None)
                .or_insert(Some(index));
        }
    }
    for (name, index) in unqualified {
        if let Some(index) = index {
            expression_indices.insert(name, index);
        }
    }
    let (value, value_indices) = match case_value {
        crate::GroupedCaseValue::Count => (None, Vec::new()),
        crate::GroupedCaseValue::Column(column) => {
            let index = super::column_index(relations, column)?;
            let kind = typed::base_type(super::column_type(relations, index));
            if !matches!(
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
            ) {
                return Err(invalid("conditional JOIN SUM requires numeric columns"));
            }
            (
                Some(crate::execute::multi_aggregate::BoundExpr::Column(index)),
                vec![index],
            )
        }
        crate::GroupedCaseValue::Expression(expression) => {
            let mut names = Vec::new();
            expression.referenced_columns(&mut names);
            let mut indices = Vec::with_capacity(names.len());
            for name in names {
                let (relation, column) = name
                    .split_once('\0')
                    .map_or(("", name.as_str()), |(relation, column)| (relation, column));
                indices.push(super::column_index(
                    relations,
                    &crate::JoinColumn {
                        relation: relation.to_owned(),
                        name: column.to_owned(),
                    },
                )?);
            }
            (
                Some(crate::execute::multi_aggregate::BoundExpr::bind(
                    expression,
                    &expression_indices,
                )?),
                indices,
            )
        }
    };
    let else_zero = matches!(
        filter,
        crate::GroupedFilter::CaseElseZero { .. }
            | crate::GroupedFilter::CaseElseZeroCombined { .. }
            | crate::GroupedFilter::CaseNull { .. }
            | crate::GroupedFilter::CaseExpression {
                else_zero: true,
                ..
            }
    );
    Ok(BoundCaseMetric {
        filter: bind_case_filter(filter, relations)?,
        value,
        value_indices,
        else_zero,
    })
}

#[derive(Clone, Default)]
struct ScalarAggregateState {
    sum: Option<super::super::Number>,
    count: u64,
    extreme: Option<super::super::Number>,
    distinct: HashSet<typed::Key>,
    distinct_bytes: usize,
}

#[derive(Default)]
struct OutputState {
    seen: u64,
    emitted: usize,
    ordered_rows: Vec<Vec<typed::Cell>>,
    scalar_sum: Option<super::super::Number>,
    scalar_aggregates: Vec<ScalarAggregateState>,
    formula_sums: Vec<Option<super::super::Number>>,
    groups: HashMap<Vec<typed::Key>, GroupRow>,
    group_key: Vec<typed::Key>,
    group_bytes: usize,
}

struct OutputSpec<'a> {
    selected: &'a [(usize, String, Option<usize>)],
    scalar_projections: Vec<super::super::multi_aggregate::BoundExpr>,
    order_by: Vec<(usize, bool, Option<bool>)>,
    /// Physical source types, including projected USING fallbacks.
    types: Vec<&'a str>,
    null_filter: Option<(usize, bool)>,
    comparisons: &'a [(usize, crate::Operator, typed::Cell)],
    column_comparisons: &'a [(usize, crate::Operator, usize)],
    substring_comparisons: &'a [BoundSubstringComparison],
    group_indices: Option<Vec<usize>>,
    window_ratio: Option<(usize, f64)>,
    window_average: Option<Vec<usize>>,
    avg_indices: Vec<usize>,
    case_metrics: Vec<BoundCaseMetric>,
    group_year: Option<usize>,
    sum: Option<&'a BoundSum>,
    scalar_sum: bool,
    scalar_aggregates: Vec<BoundScalarAggregate>,
    grouped_min: bool,
    formula: Option<&'a BoundFormula>,
    grouped_formula: Option<&'a BoundFormula>,
    grouped_sum_extras: &'a [BoundSum],
    grouped_extrema: Vec<(usize, bool)>,
    distinct_index: Option<usize>,
    row_filters: Option<&'a super::row_filters::RowFilters>,
    exists: &'a [super::exists::BoundExists],
    exists_expression: &'a [Vec<usize>],
    fact_prefiltered_exists: Vec<usize>,
    avg: Option<&'a super::correlated_avg::BoundAvg>,
    correlated_sum: Option<&'a super::correlated_sum::BoundSum>,
    offset: usize,
    limit: Option<usize>,
}

fn evaluate_formula(
    formula: &BoundFormula,
    sums: &[Option<super::super::Number>],
) -> Result<Option<super::super::Number>, ExecutionError> {
    let row = sums
        .iter()
        .map(|value| value.map_or(typed::Cell::Null, typed::Cell::Numeric))
        .collect::<Vec<_>>();
    super::super::multi_aggregate::eval_row_arithmetic(&formula.expression, &row)
}

fn compare_sum_values(
    left: Option<super::super::Number>,
    right: Option<super::super::Number>,
    descending: bool,
    nulls_first: Option<bool>,
) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => {
            let order = super::super::multi_aggregate::compare_expression_numbers(left, right)
                .unwrap_or_else(|| super::super::compare_numbers(left, right));
            if descending { order.reverse() } else { order }
        }
        (left, right) => typed::compare_order_cells(
            &left.map_or(typed::Cell::Null, typed::Cell::Numeric),
            &right.map_or(typed::Cell::Null, typed::Cell::Numeric),
            descending,
            nulls_first,
        ),
    }
}

fn number_as_f64(value: super::super::Number) -> f64 {
    match value {
        super::super::Number::Unsigned(value) => value as f64,
        super::super::Number::Signed(value) => value as f64,
        super::super::Number::Float(value) => value,
    }
}

fn write_scalar_aggregates<W: Write>(
    query: &JoinSelect,
    states: &[ScalarAggregateState],
    write_row: bool,
    output: &mut W,
) -> Result<(), ExecutionError> {
    super::write_scalar_aggregate_header(query, output)?;
    if !write_row {
        return Ok(());
    }
    for (index, (aggregate, state)) in query.scalar_aggregates.iter().zip(states).enumerate() {
        if index > 0 {
            output.write_all(b"\t")?;
        }
        let value = match &aggregate.aggregate {
            crate::JoinAggregate::CountStar | crate::JoinAggregate::CountColumn(_) => {
                Some(super::super::Number::Unsigned(u128::from(state.count)))
            }
            crate::JoinAggregate::Numeric {
                function: crate::AggregateFunction::Sum,
                ..
            } => state.sum,
            crate::JoinAggregate::Numeric {
                function: crate::AggregateFunction::Avg,
                ..
            } => state.sum.and_then(|sum| {
                (state.count > 0)
                    .then(|| super::super::Number::Float(number_as_f64(sum) / state.count as f64))
            }),
            crate::JoinAggregate::Numeric {
                function: crate::AggregateFunction::Min | crate::AggregateFunction::Max,
                ..
            } => state.extreme,
            crate::JoinAggregate::Numeric { .. } => {
                return Err(invalid("unsupported scalar JOIN aggregate"));
            }
        };
        typed::write_value(
            output,
            &value.map_or(typed::Cell::Null, typed::Cell::Numeric),
        )?;
    }
    output.write_all(b"\n")?;
    Ok(())
}

// SUBSTRING uses bounded 1-based byte offsets, matching the scalar path.
// NULL comparisons cannot satisfy WHERE; empty suffixes remain valid values.
fn substring_bytes(value: &typed::Cell, start: usize, length: usize) -> Option<&[u8]> {
    let typed::Cell::Text(bytes) = value else {
        return None;
    };
    let from = (start - 1).min(bytes.len());
    let to = from.saturating_add(length).min(bytes.len());
    let part = &bytes[from..to];
    std::str::from_utf8(part).ok()?;
    Some(part)
}

fn physical_projection(
    relation: &Relation,
    logical_columns: &[usize],
) -> Result<Vec<usize>, ExecutionError> {
    let mut physical = Vec::with_capacity(logical_columns.len());
    for &column in logical_columns {
        let source = if column < relation.physical_columns {
            column
        } else {
            relation
                .virtual_columns
                .iter()
                .find(|virtual_column| virtual_column.index == column)
                .map(|virtual_column| virtual_column.source_index)
                .ok_or_else(|| invalid("JOIN projection references an unknown virtual column"))?
        };
        if !physical.contains(&source) {
            physical.push(source);
        }
    }
    Ok(physical)
}

fn virtual_cell(
    source: &typed::Cell,
    start: usize,
    length: usize,
) -> Result<typed::Cell, ExecutionError> {
    match source {
        typed::Cell::Null => Ok(typed::Cell::Null),
        typed::Cell::Text(_) => substring_bytes(source, start, length)
            .map(|value| typed::Cell::Text(value.to_vec()))
            .ok_or_else(|| invalid("derived JOIN SUBSTRING is not valid UTF-8")),
        _ => Err(invalid("derived JOIN SUBSTRING requires a text column")),
    }
}

fn expand_projected_rows(
    rows: &mut [Vec<typed::Cell>],
    relation: &Relation,
    physical_columns: &[usize],
    logical_columns: &[usize],
) -> Result<(), ExecutionError> {
    if relation.virtual_columns.is_empty() && physical_columns == logical_columns {
        return Ok(());
    }
    for row in rows {
        let mut expanded = Vec::with_capacity(logical_columns.len());
        for &column in logical_columns {
            if column < relation.physical_columns {
                let position = physical_columns
                    .iter()
                    .position(|&physical| physical == column)
                    .ok_or_else(|| invalid("JOIN physical projection is incomplete"))?;
                expanded.push(row[position].clone());
            } else {
                let virtual_column = relation
                    .virtual_columns
                    .iter()
                    .find(|virtual_column| virtual_column.index == column)
                    .ok_or_else(|| {
                        invalid("JOIN projection references an unknown virtual column")
                    })?;
                let source_position = physical_columns
                    .iter()
                    .position(|&physical| physical == virtual_column.source_index)
                    .ok_or_else(|| invalid("JOIN virtual source was not projected"))?;
                expanded.push(virtual_cell(
                    &row[source_position],
                    virtual_column.start,
                    virtual_column.length,
                )?);
            }
        }
        *row = expanded;
    }
    Ok(())
}

fn passes_substring(
    row: &[typed::Cell],
    &(left, left_start, left_len, operator, right, right_start, right_len):
        &BoundSubstringComparison,
) -> bool {
    let (Some(left), Some(right)) = (
        substring_bytes(&row[left], left_start, left_len),
        substring_bytes(&row[right], right_start, right_len),
    ) else {
        return false;
    };
    let ordering = left.cmp(right);
    match operator {
        crate::Operator::Eq => ordering.is_eq(),
        crate::Operator::Ne => ordering.is_ne(),
        crate::Operator::Lt => ordering.is_lt(),
        crate::Operator::Le => ordering.is_le(),
        crate::Operator::Gt => ordering.is_gt(),
        crate::Operator::Ge => ordering.is_ge(),
        _ => false,
    }
}

fn fact_side_comparisons(
    comparisons: &[(usize, crate::Operator, typed::Cell)],
    fact_width: usize,
) -> Vec<(usize, crate::Operator, typed::Cell)> {
    comparisons
        .iter()
        .filter(|(index, _, _)| *index < fact_width)
        .cloned()
        .collect()
}

fn passes_fact_comparisons(
    row: &[typed::Cell],
    comparisons: &[(usize, crate::Operator, typed::Cell)],
) -> bool {
    comparisons.iter().all(|(index, operator, literal)| {
        typed::matches_comparison(&row[*index], literal, *operator)
    })
}

fn group_value(row: &[typed::Cell], index: usize, year: bool) -> typed::Cell {
    if !year {
        return row[index].clone();
    }
    let days = match row[index] {
        typed::Cell::Numeric(super::super::Number::Unsigned(days)) => days as i64,
        typed::Cell::Numeric(super::super::Number::Signed(days)) => days as i64,
        _ => return typed::Cell::Null,
    };
    typed::Cell::Numeric(super::super::Number::Signed(i128::from(
        crate::date_scalar::civil_from_days(days).0,
    )))
}

fn visit<W: Write>(
    depth: usize,
    row: &mut Vec<typed::Cell>,
    stages: &[Stage],
    scratch_keys: &mut [Vec<typed::Key>],
    spec: &OutputSpec<'_>,
    state: &mut OutputState,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if spec.limit.is_some_and(|limit| state.emitted >= limit) {
        return Ok(());
    }
    if depth == stages.len() {
        if let Some((column, not_null)) = spec.null_filter {
            let is_null = matches!(row[column], typed::Cell::Null);
            if is_null == not_null {
                return Ok(());
            }
        }
        if !spec.comparisons.iter().all(|(index, operator, literal)| {
            typed::matches_comparison(&row[*index], literal, *operator)
        }) || !spec
            .column_comparisons
            .iter()
            .all(|(left, operator, right)| {
                typed::matches_comparison(&row[*left], &row[*right], *operator)
            })
        {
            return Ok(());
        }
        if !spec
            .substring_comparisons
            .iter()
            .all(|filter| passes_substring(row, filter))
        {
            return Ok(());
        }
        if let Some(filters) = spec.row_filters {
            if !filters.passes(row)? {
                return Ok(());
            }
        }
        if !super::exists::passes_expression(
            spec.exists_expression,
            spec.exists,
            &spec.fact_prefiltered_exists,
            row,
        )? {
            return Ok(());
        }
        if let Some(avg) = spec.avg {
            if !avg.passes(row)? {
                return Ok(());
            }
        }
        if let Some(sum) = spec.correlated_sum {
            if !sum.passes(row)? {
                return Ok(());
            }
        }
        if !spec.scalar_aggregates.is_empty() {
            for (index, metric) in spec.scalar_aggregates.iter().enumerate() {
                let state = &mut state.scalar_aggregates[index];
                let value = metric.column.map(|column| &row[column]);
                if metric.function == crate::AggregateFunction::Count {
                    if metric.distinct {
                        if let Some(value) =
                            value.filter(|value| !matches!(value, typed::Cell::Null))
                        {
                            let key = value.key();
                            if !state.distinct.contains(&key) {
                                let payload = match &key {
                                    typed::Key::Text(value) => value.len(),
                                    typed::Key::Uuid(value) => value.len(),
                                    _ => 0,
                                };
                                let charge = std::mem::size_of::<typed::Key>()
                                    .checked_add(96)
                                    .and_then(|bytes| bytes.checked_add(payload))
                                    .ok_or_else(|| invalid("scalar JOIN DISTINCT size overflow"))?;
                                state.distinct_bytes = state
                                    .distinct_bytes
                                    .checked_add(charge)
                                    .filter(|&bytes| bytes <= MAX_STREAM_GROUP_BYTES)
                                    .ok_or_else(|| {
                                        invalid("scalar JOIN DISTINCT memory budget exceeded")
                                    })?;
                                state.distinct.insert(key);
                                state.count = state
                                    .count
                                    .checked_add(1)
                                    .ok_or_else(|| invalid("scalar JOIN COUNT overflow"))?;
                            }
                        }
                    } else if value.is_none_or(|value| !matches!(value, typed::Cell::Null)) {
                        state.count = state
                            .count
                            .checked_add(1)
                            .ok_or_else(|| invalid("scalar JOIN COUNT overflow"))?;
                    }
                    continue;
                }
                let Some(value) = value else {
                    return Err(invalid("scalar JOIN aggregate requires a column"));
                };
                let typed::Cell::Numeric(value) = value else {
                    if matches!(value, typed::Cell::Null) {
                        continue;
                    }
                    return Err(invalid("scalar JOIN aggregate requires numeric values"));
                };
                match metric.function {
                    crate::AggregateFunction::Sum | crate::AggregateFunction::Avg => {
                        state.sum = Some(match state.sum {
                            Some(previous) => {
                                super::super::checked_aggregate_sum(previous, *value)?
                            }
                            None => *value,
                        });
                        state.count = state
                            .count
                            .checked_add(1)
                            .ok_or_else(|| invalid("scalar JOIN aggregate count overflow"))?;
                    }
                    crate::AggregateFunction::Min | crate::AggregateFunction::Max => {
                        state.extreme = Some(match state.extreme {
                            Some(previous) => {
                                let order =
                                    super::super::multi_aggregate::compare_expression_numbers(
                                        *value, previous,
                                    )
                                    .unwrap_or_else(|| {
                                        super::super::compare_numbers(*value, previous)
                                    });
                                let replace = if metric.function == crate::AggregateFunction::Min {
                                    order.is_lt()
                                } else {
                                    order.is_gt()
                                };
                                if replace { *value } else { previous }
                            }
                            None => *value,
                        });
                    }
                    _ => return Err(invalid("unsupported scalar JOIN aggregate")),
                }
            }
            return Ok(());
        }
        if let Some(formula) = spec.formula {
            for (index, sum) in formula.sums.iter().enumerate() {
                if let Some(value) =
                    super::super::multi_aggregate::eval_row_arithmetic(&sum.expression, row)?
                {
                    state.formula_sums[index] = Some(match state.formula_sums[index] {
                        Some(previous) => super::super::checked_aggregate_sum(previous, value)?,
                        None => value,
                    });
                }
            }
            return Ok(());
        }
        if spec.scalar_sum {
            if let Some(value) = super::super::multi_aggregate::eval_row_arithmetic(
                &spec.sum.expect("bound SUM").expression,
                row,
            )? {
                state.scalar_sum = Some(match state.scalar_sum {
                    Some(previous) => super::super::checked_aggregate_sum(previous, value)?,
                    None => value,
                });
            }
            return Ok(());
        }
        if let Some(indices) = &spec.group_indices {
            state.group_key.clear();
            for (position, &index) in indices.iter().enumerate() {
                let key = if spec.group_year == Some(position) {
                    group_value(row, index, true).key()
                } else {
                    row[index].key()
                };
                state.group_key.push(key);
            }
            let value = match spec.sum {
                Some(sum) => {
                    super::super::multi_aggregate::eval_row_arithmetic(&sum.expression, row)?
                }
                None => None,
            };
            let mut average_values = [None; 8];
            for (index, &column) in spec.avg_indices.iter().enumerate() {
                average_values[index] = match &row[column] {
                    typed::Cell::Numeric(value) => Some(*value),
                    typed::Cell::Null => None,
                    _ => return Err(invalid("grouped AVG requires numeric values")),
                };
            }
            let case_values = spec
                .case_metrics
                .iter()
                .map(|metric| {
                    metric
                        .filter
                        .passes(row)
                        .and_then(|matched| metric.value_for_row(matched, row))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut formula_values = [None; 8];
            if let Some(formula) = spec.grouped_formula {
                for (index, sum) in formula.sums.iter().enumerate() {
                    formula_values[index] =
                        super::super::multi_aggregate::eval_row_arithmetic(&sum.expression, row)?;
                }
            }
            let mut extra_sum_values = [None; 8];
            for (index, sum) in spec.grouped_sum_extras.iter().enumerate() {
                extra_sum_values[index] =
                    super::super::multi_aggregate::eval_row_arithmetic(&sum.expression, row)?;
            }
            let mut extreme_values = [None; 16];
            for (index, &(column, _)) in spec.grouped_extrema.iter().enumerate() {
                extreme_values[index] = match row.get(column) {
                    Some(typed::Cell::Numeric(value)) => Some(*value),
                    Some(typed::Cell::Null) => None,
                    _ => return Err(invalid("grouped MIN/MAX requires numeric values")),
                };
            }
            if let Some((_, count, sum, _, sums, avgs, cases, _)) =
                state.groups.get_mut(state.group_key.as_slice())
            {
                if let Some(avgs) = avgs {
                    for (index, value) in average_values
                        .iter()
                        .enumerate()
                        .take(spec.avg_indices.len())
                    {
                        if let Some(value) = value {
                            avgs[index].0 = Some(match avgs[index].0 {
                                Some(previous) => {
                                    super::super::checked_aggregate_sum(previous, *value)?
                                }
                                None => *value,
                            });
                            avgs[index].1 = avgs[index]
                                .1
                                .checked_add(1)
                                .ok_or_else(|| invalid("grouped AVG count overflow"))?;
                        }
                    }
                }
                if let Some(formula) = spec.grouped_formula {
                    let sums = sums.as_mut().expect("grouped formula sums exist");
                    for (index, value) in formula_values.iter().enumerate().take(formula.sums.len())
                    {
                        if let Some(value) = value {
                            sums[index] = Some(match sums[index] {
                                Some(previous) => {
                                    super::super::checked_aggregate_sum(previous, *value)?
                                }
                                None => *value,
                            });
                        }
                    }
                }
                if !spec.grouped_sum_extras.is_empty() || !spec.grouped_extrema.is_empty() {
                    let sums = sums.as_mut().expect("grouped SUM states exist");
                    for (index, value) in extra_sum_values
                        .iter()
                        .enumerate()
                        .take(spec.grouped_sum_extras.len())
                    {
                        if let Some(value) = value {
                            sums[index] = Some(match sums[index] {
                                Some(previous) => {
                                    super::super::checked_aggregate_sum(previous, *value)?
                                }
                                None => *value,
                            });
                        }
                    }
                    for (index, (value, &(_, is_min))) in
                        extreme_values.iter().zip(&spec.grouped_extrema).enumerate()
                    {
                        if let Some(value) = value {
                            let slot = &mut sums[spec.grouped_sum_extras.len() + index];
                            *slot = Some(match *slot {
                                Some(previous)
                                    if (is_min
                                        && super::super::compare_numbers(*value, previous)
                                            .is_lt())
                                        || (!is_min
                                            && super::super::compare_numbers(*value, previous)
                                                .is_gt()) =>
                                {
                                    *value
                                }
                                Some(previous) => previous,
                                None => *value,
                            });
                        }
                    }
                }
                if let Some(cases) = cases {
                    for (index, value) in case_values.iter().enumerate() {
                        if let Some(value) = value {
                            cases[index] = Some(match cases[index] {
                                Some(previous) => {
                                    super::super::checked_aggregate_sum(previous, *value)?
                                }
                                None => *value,
                            });
                        }
                    }
                }
                if let Some(value) = value {
                    *sum = Some(match *sum {
                        Some(previous) if spec.grouped_min => {
                            if super::super::compare_numbers(value, previous).is_lt() {
                                value
                            } else {
                                previous
                            }
                        }
                        Some(previous) => super::super::checked_aggregate_sum(previous, value)?,
                        None => value,
                    });
                }
                if spec.distinct_index.is_none() {
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| invalid("chained JOIN group count overflow"))?;
                }
            } else {
                if state.groups.len() >= MAX_STREAM_GROUPS {
                    return Err(invalid("chained JOIN supports at most 100000 groups"));
                }
                let variable = indices.iter().try_fold(0usize, |total, &index| {
                    let len = match &row[index] {
                        typed::Cell::Text(value) => value.len(),
                        typed::Cell::Uuid(value) => value.len(),
                        _ => 0,
                    };
                    len.checked_mul(2)
                        .and_then(|bytes| total.checked_add(bytes))
                        .ok_or_else(|| invalid("chained JOIN grouped key size overflow"))
                })?;
                let fixed = indices
                    .len()
                    .checked_mul(
                        std::mem::size_of::<typed::Key>() + std::mem::size_of::<typed::Cell>(),
                    )
                    .and_then(|bytes| {
                        bytes.checked_add(std::mem::size_of::<(
                            Vec<typed::Key>,
                            Vec<typed::Cell>,
                            u64,
                            Option<super::super::Number>,
                            Option<HashSet<typed::Key>>,
                            Option<Box<[Option<super::super::Number>]>>,
                            Option<Box<[(Option<super::super::Number>, u64)]>>,
                            Option<Box<[Option<super::super::Number>]>>,
                            Option<super::super::Number>,
                        )>())
                    })
                    .ok_or_else(|| invalid("chained JOIN group size overflow"))?;
                state.group_bytes = state
                    .group_bytes
                    .checked_add(fixed)
                    .and_then(|bytes| bytes.checked_add(variable))
                    .and_then(|bytes| {
                        let extra = spec.grouped_formula.map_or(0, |formula| {
                            16 + formula.sums.len()
                                * std::mem::size_of::<Option<super::super::Number>>()
                        });
                        let grouped_sum_count =
                            spec.grouped_sum_extras.len() + spec.grouped_extrema.len();
                        let grouped_sums_extra = if grouped_sum_count == 0 {
                            0
                        } else {
                            16 + grouped_sum_count
                                * std::mem::size_of::<Option<super::super::Number>>()
                        };
                        let avg_extra = if spec.avg_indices.is_empty() {
                            0
                        } else {
                            16 + spec.avg_indices.len()
                                * std::mem::size_of::<(Option<super::super::Number>, u64)>()
                        };
                        let case_extra = if spec.case_metrics.is_empty() {
                            0
                        } else {
                            16 + spec.case_metrics.len()
                                * std::mem::size_of::<Option<super::super::Number>>()
                        };
                        bytes
                            .checked_add(extra)
                            .and_then(|bytes| bytes.checked_add(grouped_sums_extra))
                            .and_then(|bytes| bytes.checked_add(avg_extra))
                            .and_then(|bytes| bytes.checked_add(case_extra))
                    })
                    .filter(|&bytes| bytes <= MAX_STREAM_GROUP_BYTES)
                    .ok_or_else(|| invalid("chained JOIN grouped memory budget exceeded"))?;
                state.groups.insert(
                    state.group_key.clone(),
                    (
                        indices
                            .iter()
                            .enumerate()
                            .map(|(position, &index)| {
                                group_value(row, index, spec.group_year == Some(position))
                            })
                            .collect(),
                        if spec.distinct_index.is_some() { 0 } else { 1 },
                        value,
                        spec.distinct_index.map(|_| HashSet::new()),
                        spec.grouped_formula
                            .map(|formula| {
                                formula_values[..formula.sums.len()]
                                    .to_vec()
                                    .into_boxed_slice()
                            })
                            .or_else(|| {
                                let count =
                                    spec.grouped_sum_extras.len() + spec.grouped_extrema.len();
                                (count > 0).then(|| {
                                    let mut values = vec![None; count];
                                    values[..spec.grouped_sum_extras.len()].copy_from_slice(
                                        &extra_sum_values[..spec.grouped_sum_extras.len()],
                                    );
                                    for (index, value) in extreme_values
                                        .iter()
                                        .take(spec.grouped_extrema.len())
                                        .enumerate()
                                    {
                                        values[spec.grouped_sum_extras.len() + index] = *value;
                                    }
                                    values.into_boxed_slice()
                                })
                            }),
                        (!spec.avg_indices.is_empty()).then(|| {
                            average_values[..spec.avg_indices.len()]
                                .iter()
                                .map(|value| (*value, u64::from(value.is_some())))
                                .collect::<Vec<_>>()
                                .into_boxed_slice()
                        }),
                        (!spec.case_metrics.is_empty())
                            .then(|| case_values.clone().into_boxed_slice()),
                        None,
                    ),
                );
            }
            if let Some(index) = spec.distinct_index {
                if !matches!(row[index], typed::Cell::Null) {
                    let key = row[index].key();
                    let (_, count, _, distinct, _, _, _, _) = state
                        .groups
                        .get_mut(state.group_key.as_slice())
                        .expect("group exists");
                    let distinct = distinct.as_mut().expect("distinct set exists");
                    if !distinct.contains(&key) {
                        let charge = std::mem::size_of::<typed::Key>()
                            .saturating_mul(2)
                            .saturating_add(64);
                        state.group_bytes = state
                            .group_bytes
                            .checked_add(charge)
                            .filter(|&bytes| bytes <= MAX_STREAM_GROUP_BYTES)
                            .ok_or_else(|| {
                                invalid("chained JOIN grouped memory budget exceeded")
                            })?;
                        distinct.insert(key);
                        *count = count
                            .checked_add(1)
                            .ok_or_else(|| invalid("chained JOIN distinct count overflow"))?;
                    }
                }
            }
            return Ok(());
        }
        state.seen = state
            .seen
            .checked_add(1)
            .ok_or_else(|| invalid("JOIN matched-row count overflow"))?;
        if !spec.order_by.is_empty() {
            if state.ordered_rows.len() >= super::MAX_RESULT_ROWS {
                return Err(invalid("JOIN produces more than 131072 rows"));
            }
            let mut ordered = row.clone();
            if spec
                .order_by
                .iter()
                .any(|(index, _, _)| *index >= spec.types.len())
            {
                for expression in &spec.scalar_projections {
                    ordered.push(super::super::multi_aggregate::eval_row_value(
                        expression, row,
                    )?);
                }
            }
            state.ordered_rows.push(ordered);
            return Ok(());
        }
        // Output is written directly to the caller, not kept in an
        // intermediate row vector. No materialized-join result cap applies.
        if state.seen <= spec.offset as u64 {
            return Ok(());
        }
        for (position, (index, _, fallback)) in spec.selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            if let Some(expression) =
                super::scalar_output_index(*index, spec.scalar_projections.len())
            {
                let value = super::super::multi_aggregate::eval_row_value(
                    &spec.scalar_projections[expression],
                    row,
                )?;
                typed::write_value(output, &value)?;
                continue;
            }
            let source = if matches!(row[*index], typed::Cell::Null) {
                fallback.unwrap_or(*index)
            } else {
                *index
            };
            typed::write_typed_value(output, &row[source], spec.types[source])?;
        }
        output.write_all(b"\n")?;
        state.emitted += 1;
        return Ok(());
    }

    let stage = &stages[depth];
    // Preserve per-stage key buffers across millions of fact rows.
    let has_key = row_key_into(row, &stage.left_keys, &mut scratch_keys[depth]);
    let matches = has_key
        .then(|| stage.hash.lookup(scratch_keys[depth].as_slice()))
        .flatten();
    let Some(matches) = matches else {
        if stage.kind == JoinType::Left {
            let original_len = row.len();
            row.resize(original_len + stage.full_width, typed::Cell::Null);
            let result = visit(depth + 1, row, stages, scratch_keys, spec, state, output);
            row.truncate(original_len);
            result?;
        }
        return Ok(());
    };
    for &matched in matches.iter().take(if stage.any { 1 } else { usize::MAX }) {
        let original_len = row.len();
        row.resize(original_len + stage.full_width, typed::Cell::Null);
        for (&column, value) in stage.projected_columns.iter().zip(&stage.rows[matched]) {
            row[original_len + column] = value.clone();
        }
        let result = visit(depth + 1, row, stages, scratch_keys, spec, state, output);
        row.truncate(original_len);
        result?;
        if spec.limit.is_some_and(|limit| state.emitted >= limit) {
            break;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    relations: &[Relation],
    initial_step: &JoinStep,
    comparisons: &[(usize, crate::Operator, typed::Cell)],
    column_comparisons: &[(usize, crate::Operator, usize)],
    substring_comparisons: &[BoundSubstringComparison],
    sum: Option<&BoundSum>,
    grouped_sum_extras: &[BoundSum],
    formula: Option<&BoundFormula>,
    distinct_index: Option<usize>,
    row_filters: Option<&super::row_filters::RowFilters>,
    exists: &[super::exists::BoundExists],
    exists_expression: &[Vec<usize>],
    avg: Option<&super::correlated_avg::BoundAvg>,
    correlated_sum: Option<&super::correlated_sum::BoundSum>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if relations[0].names.is_empty() {
        return Err(invalid("JOIN requires nonempty table schemas"));
    }
    let selected = if query.scalar_sum.is_some()
        || query.scalar_formula.is_some()
        || !query.scalar_aggregates.is_empty()
    {
        Vec::new()
    } else {
        output_columns(query, relations, initial_step)?
    };
    let null_filter = query
        .where_null
        .as_ref()
        .map(|(column, not_null)| column_index(relations, column).map(|index| (index, *not_null)))
        .transpose()?;
    let avg_indices = query
        .grouped_avg_metrics
        .iter()
        .map(|(column, _)| {
            let index = column_index(relations, column)?;
            let kind = typed::base_type(column_type(relations, index));
            if !matches!(
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
            ) {
                return Err(invalid("grouped JOIN AVG requires numeric columns"));
            }
            Ok(index)
        })
        .collect::<Result<Vec<_>, ExecutionError>>()?;
    let mut grouped_extrema = Vec::new();
    for (column, _) in query.grouped_min_metrics.iter().skip(1) {
        let index = column_index(relations, column)?;
        let kind = typed::base_type(column_type(relations, index));
        if !matches!(
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
        ) {
            return Err(invalid("grouped JOIN MIN requires numeric columns"));
        }
        grouped_extrema.push((index, true));
    }
    for (column, _) in &query.grouped_max_metrics {
        let index = column_index(relations, column)?;
        let kind = typed::base_type(column_type(relations, index));
        if !matches!(
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
        ) {
            return Err(invalid("grouped JOIN MAX requires numeric columns"));
        }
        grouped_extrema.push((index, false));
    }
    let case_metrics = query
        .grouped_case_counts
        .iter()
        .map(|(filter, value, _)| bind_case_metric(filter, value, relations))
        .collect::<Result<Vec<_>, _>>()?;
    let window_ratio = if let Some(ratio) = &query.grouped_window_ratio {
        let keys = if query.grouped_execution_keys.is_empty() {
            query
                .grouped_count
                .as_ref()
                .ok_or_else(|| invalid("grouped window ratio requires group keys"))?
        } else {
            &query.grouped_execution_keys
        };
        let partition = keys
            .iter()
            .position(|column| column == &ratio.partition)
            .ok_or_else(|| invalid("unknown grouped window partition key"))?;
        let multiplier = ratio
            .multiplier
            .parse::<f64>()
            .map_err(|_| invalid("invalid grouped window ratio multiplier"))?;
        if !multiplier.is_finite() {
            return Err(invalid("invalid grouped window ratio multiplier"));
        }
        Some((partition, multiplier))
    } else {
        None
    };
    let window_average = query
        .grouped_window_average
        .as_ref()
        .map(|average| {
            let keys = if query.grouped_execution_keys.is_empty() {
                query
                    .grouped_count
                    .as_ref()
                    .ok_or_else(|| invalid("grouped window AVG requires group keys"))?
            } else {
                &query.grouped_execution_keys
            };
            average
                .partition
                .iter()
                .map(|partition| {
                    keys.iter()
                        .position(|key| key == partition)
                        .ok_or_else(|| invalid("unknown grouped window AVG partition key"))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let group_indices = if query.grouped_count.is_some() {
        let keys = if query.grouped_execution_keys.is_empty() {
            if query.scalar_projections.is_empty() {
                selected.iter().map(|(index, _, _)| *index).collect()
            } else {
                query
                    .columns
                    .as_ref()
                    .ok_or_else(|| invalid("grouped JOIN requires projected keys"))?
                    .iter()
                    .map(|key| column_index(relations, key))
                    .collect::<Result<Vec<_>, _>>()?
            }
        } else {
            query
                .grouped_execution_keys
                .iter()
                .map(|key| column_index(relations, key))
                .collect::<Result<Vec<_>, _>>()?
        };
        Some(keys)
    } else {
        None
    };
    let scalar_aggregates = query
        .scalar_aggregates
        .iter()
        .map(|item| {
            let (function, column) = match &item.aggregate {
                crate::JoinAggregate::CountStar => (crate::AggregateFunction::Count, None),
                crate::JoinAggregate::CountColumn(column) => (
                    crate::AggregateFunction::Count,
                    Some(column_index(relations, column)?),
                ),
                crate::JoinAggregate::Numeric { function, column } => {
                    (*function, Some(column_index(relations, column)?))
                }
            };
            if let Some(index) = column {
                let kind = typed::base_type(column_type(relations, index));
                let numeric = matches!(
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
                );
                if function != crate::AggregateFunction::Count && !numeric {
                    return Err(invalid("scalar JOIN aggregate requires numeric columns"));
                }
            }
            Ok(BoundScalarAggregate {
                function,
                column,
                distinct: item.distinct,
            })
        })
        .collect::<Result<Vec<_>, ExecutionError>>()?;
    let scalar_projections = super::bind_scalar_projections(query, relations)?;
    let scalar_order_base = relations
        .iter()
        .map(|relation| relation.names.len())
        .sum::<usize>();
    let order_by = if query.scalar_projections.is_empty() || query.grouped_count.is_some() {
        Vec::new()
    } else {
        query
            .order_by
            .iter()
            .map(|(column, descending, nulls_first)| {
                let index = column
                    .scalar_order_index()
                    .map(|index| scalar_order_base + index)
                    .map_or_else(|| column_index(relations, column), Ok)?;
                Ok((index, *descending, *nulls_first))
            })
            .collect::<Result<Vec<_>, ExecutionError>>()?
    };
    let fact_prefiltered_exists =
        super::exists::fact_prefilters(exists, exists_expression, relations[0].names.len())?;
    let spec = OutputSpec {
        selected: &selected,
        scalar_projections,
        order_by,
        types: relations
            .iter()
            .flat_map(|relation| relation.types.iter().map(String::as_str))
            .collect(),
        null_filter,
        comparisons,
        column_comparisons,
        substring_comparisons,
        sum,
        scalar_sum: query.scalar_sum.is_some(),
        scalar_aggregates,
        grouped_min: query.grouped_min.is_some(),
        formula: if query.scalar_formula.is_some() {
            formula
        } else {
            None
        },
        grouped_formula: if query.grouped_formula.is_some() {
            formula
        } else {
            None
        },
        grouped_sum_extras,
        grouped_extrema,
        distinct_index,
        row_filters,
        exists,
        exists_expression,
        fact_prefiltered_exists,
        avg,
        correlated_sum,
        group_year: query.grouped_year.as_ref().map(|(position, _)| *position),
        group_indices,
        window_ratio,
        window_average,
        avg_indices,
        case_metrics,
        offset: query.offset,
        limit: query.limit,
    };
    // Project all dimension columns needed by any later stage, output
    // expression, or final filter. Preserve the original logical indexes.
    let width = relations
        .last()
        .map(|relation| relation.start + relation.names.len())
        .unwrap_or(0);
    let mut referenced = vec![false; width];
    for (step_index, step) in std::iter::once(initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let relation = &relations[step_index + 1];
        let (left, right) = step_keys(step, &relations[..=step_index + 1], relation.start)?;
        for index in left {
            referenced[index] = true;
        }
        for index in right {
            referenced[relation.start + index] = true;
        }
    }
    for &(index, _, fallback) in &selected {
        if super::scalar_output_index(index, query.scalar_projections.len()).is_some() {
            continue;
        }
        referenced[index] = true;
        if let Some(index) = fallback {
            referenced[index] = true;
        }
    }
    for projection in &query.scalar_projections {
        let mut names = Vec::new();
        projection.expression.referenced_columns(&mut names);
        for name in names {
            let (relation, column) = name
                .split_once('\0')
                .map_or(("", name.as_str()), |(relation, column)| (relation, column));
            referenced[column_index(
                relations,
                &crate::JoinColumn {
                    relation: relation.to_owned(),
                    name: column.to_owned(),
                },
            )?] = true;
        }
    }
    for key in &query.grouped_execution_keys {
        referenced[column_index(relations, key)?] = true;
    }
    if let Some((index, _)) = null_filter {
        referenced[index] = true;
    }
    for (index, _, _) in comparisons {
        referenced[*index] = true;
    }
    for (left, _, right) in column_comparisons {
        referenced[*left] = true;
        referenced[*right] = true;
    }
    for &(left, _, _, _, right, _, _) in substring_comparisons {
        referenced[left] = true;
        referenced[right] = true;
    }
    if let Some(sum) = sum {
        for &index in &sum.indices {
            referenced[index] = true;
        }
    }
    for sum in grouped_sum_extras {
        for &index in &sum.indices {
            referenced[index] = true;
        }
    }
    if let Some(formula) = formula {
        for sum in &formula.sums {
            for &index in &sum.indices {
                referenced[index] = true;
            }
        }
    }
    for metric in &spec.scalar_aggregates {
        if let Some(index) = metric.column {
            referenced[index] = true;
        }
    }
    if let Some(index) = distinct_index {
        referenced[index] = true;
    }
    for &index in &spec.avg_indices {
        referenced[index] = true;
    }
    for &(index, _) in &spec.grouped_extrema {
        referenced[index] = true;
    }
    for metric in &spec.case_metrics {
        for &index in metric.referenced_indices() {
            referenced[index] = true;
        }
    }
    if let Some(filters) = row_filters {
        for &index in &filters.indices {
            referenced[index] = true;
        }
    }
    for filter in exists {
        referenced[filter.equality] = true;
        if let Some(index) = filter.inequality {
            referenced[index] = true;
        }
    }
    if let Some(avg) = avg {
        referenced[avg.outer_key] = true;
        referenced[avg.outer_value] = true;
    }
    if let Some(sum) = correlated_sum {
        for &index in &sum.outer_keys {
            referenced[index] = true;
        }
        referenced[sum.outer_value] = true;
    }
    let projections = relations
        .iter()
        .map(|relation| {
            let mut positions = (0..relation.names.len())
                .filter(|&index| referenced[relation.start + index])
                .collect::<Vec<_>>();
            if positions.is_empty() && !relation.names.is_empty() {
                positions.push(0);
            }
            positions
        })
        .collect::<Vec<_>>();

    let mut stages = Vec::with_capacity(relations.len() - 1);
    let mut retained = 0usize;
    let inner_join_plan = initial_step.kind == JoinType::Inner
        && query.steps.iter().all(|step| step.kind == JoinType::Inner);
    for (step_index, step) in std::iter::once(initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let relation = &relations[step_index + 1];
        let (left_keys, full_right_keys) =
            step_keys(step, &relations[..=step_index + 1], relation.start)?;
        let projected_columns = &projections[step_index + 1];
        let physical_columns = physical_projection(relation, projected_columns)?;
        let right_keys = full_right_keys
            .iter()
            .map(|column| {
                projected_columns
                    .iter()
                    .position(|projected| projected == column)
                    .ok_or_else(|| invalid("JOIN key was not projected from build relation"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let available = MAX_CHAIN_BYTES.saturating_sub(retained);
        // Safe relation-local predicates can reject build rows before hash
        // retention. The full filter still runs in the final visitor.
        let dimension_filter = inner_join_plan
            .then(|| {
                comparisons.iter().find(|(index, _, _)| {
                    index
                        .checked_sub(relation.start)
                        .is_some_and(|local| local < relation.physical_columns)
                })
            })
            .flatten();
        let row_filter = row_filters
            .filter(|_| inner_join_plan && relation.virtual_columns.is_empty())
            .and_then(|filters| {
                filters
                    .prefilter_conditions(relation.start, relation.names.len())
                    .map(|conditions| (filters, conditions))
            });
        let mut right_rows = if let Some((index, operator, value)) = dimension_filter {
            read_filtered_compact_projected_build_rows(
                &paths[step_index + 1],
                &relation.names,
                &physical_columns,
                relation.rows,
                crate::join::MAX_STREAM_JOIN_BUILD_ROWS,
                available.min(MAX_STREAM_BUILD_BYTES),
                (*index - relation.start, *operator, value),
            )?
        } else if let Some((filters, conditions)) = row_filter {
            let mut predicate =
                |row: &[typed::Cell]| filters.passes_relation(row, relation.start, &conditions);
            read_compact_projected_build_rows_matching(
                &paths[step_index + 1],
                &relation.names,
                &physical_columns,
                relation.rows,
                crate::join::MAX_STREAM_JOIN_BUILD_ROWS,
                available.min(MAX_STREAM_BUILD_BYTES),
                &mut predicate,
            )?
        } else {
            read_compact_projected_build_rows(
                &paths[step_index + 1],
                &relation.names,
                &physical_columns,
                relation.rows,
                crate::join::MAX_STREAM_JOIN_BUILD_ROWS,
                available.min(MAX_STREAM_BUILD_BYTES),
            )?
        };
        expand_projected_rows(
            &mut right_rows,
            relation,
            &physical_columns,
            projected_columns,
        )?;
        for row in &right_rows {
            retained = retained
                .checked_add(retained_row_bytes(row)?)
                .ok_or_else(|| invalid("JOIN build size overflow"))?;
        }
        if retained > MAX_CHAIN_BYTES {
            return Err(invalid("JOIN exceeds its retained-row memory budget"));
        }
        // A skewed build can contain far fewer distinct keys than input
        // rows. Reserve a small table, then charge each retained key.
        let single_numeric = single_numeric_join_key(&right_rows, &right_keys);
        let unsigned_pair = unsigned_composite_join_key(&right_rows, &right_keys);
        let hash_capacity = right_rows.len().min(4096);
        let mut hash = if unsigned_pair {
            JoinHash::with_unsigned_pair_capacity(hash_capacity)
        } else {
            JoinHash::with_capacity(right_keys.len(), hash_capacity, single_numeric)
        };
        let mut scratch_key = Vec::with_capacity(right_keys.len());
        for (index, row) in right_rows.iter().enumerate() {
            if !row_key_into(row, &right_keys, &mut scratch_key) {
                continue;
            }
            if let Some(bucket) = hash.lookup_mut(scratch_key.as_slice()) {
                if !step.any {
                    // Promotion adds a boxed Vec header and two indices; later
                    // pushes include one index and conservative spare capacity.
                    let growth_slots = if bucket.len() == 1 { 5 } else { 3 };
                    charge_hash_build(&mut retained, std::mem::size_of::<usize>() * growth_slots)?;
                    bucket.push(index);
                }
            } else {
                charge_hash_build(&mut retained, hash_key_bytes(&scratch_key, single_numeric)?)?;
                hash.insert_new(scratch_key.as_slice(), index);
            }
        }
        stages.push(Stage {
            rows: right_rows,
            hash,
            left_keys,
            projected_columns: projected_columns.clone(),
            full_width: relation.names.len(),
            kind: step.kind,
            any: step.any,
        });
    }
    if spec.group_indices.is_none()
        && !spec.scalar_sum
        && spec.scalar_aggregates.is_empty()
        && spec.formula.is_none()
    {
        write_join_header(output, &selected)?;
    }
    // A build can contain physical rows but no legal JOIN keys (only NULL
    // or NaN). In an INNER chain that is as empty as a zero-row dimension,
    // so do not read or decode the potentially enormous fact table.
    if inner_join_plan && stages.iter().any(|stage| stage.hash.is_empty()) {
        if query.grouped_count.is_some() {
            write_group_header(query, &selected, output)?;
        } else if let Some(formula) = spec.formula {
            let value = evaluate_formula(formula, &vec![None; formula.sums.len()])?;
            write_scalar_sum(query, value, true, output)?;
        } else if spec.scalar_sum {
            write_scalar_sum(query, None, true, output)?;
        } else if !spec.scalar_aggregates.is_empty() {
            write_scalar_aggregates(
                query,
                &vec![ScalarAggregateState::default(); spec.scalar_aggregates.len()],
                true,
                output,
            )?;
        }
        return Ok(());
    }
    let mut scratch_keys = stages
        .iter()
        .map(|stage| Vec::with_capacity(stage.left_keys.len()))
        .collect::<Vec<Vec<typed::Key>>>();
    // Reuse the same dependency projection for the fact scan.
    let fact_width = relations[0].names.len();
    let fact_comparisons = fact_side_comparisons(comparisons, fact_width);
    let fact_row_filter = row_filters.filter(|_| inner_join_plan).and_then(|filters| {
        filters
            .prefilter_conditions(0, fact_width)
            .map(|conditions| (filters, conditions))
    });
    let projected = &projections[0];
    let physical_projected = physical_projection(&relations[0], projected)?;
    let projected_names = physical_projected
        .iter()
        .map(|&index| relations[0].names[index].clone())
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(&paths[0], &projected_names)?;
    let mut scanned = 0u64;
    let mut state = OutputState::default();
    state
        .scalar_aggregates
        .resize_with(spec.scalar_aggregates.len(), ScalarAggregateState::default);
    if let Some(formula) = spec.formula {
        state.formula_sums.resize(formula.sums.len(), None);
    }
    // The recursive visitor restores fact row length after each match.
    // Retain this allocation across the entire scan.
    let mut row = vec![typed::Cell::Null; fact_width];
    while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            scanned = scanned
                .checked_add(1)
                .ok_or_else(|| invalid("JOIN input row count overflow"))?;
            for (position, &column) in physical_projected.iter().enumerate() {
                row[column] = typed::cell(&batch[position], index);
            }
            for virtual_column in &relations[0].virtual_columns {
                if projected.contains(&virtual_column.index) {
                    row[virtual_column.index] = virtual_cell(
                        &row[virtual_column.source_index],
                        virtual_column.start,
                        virtual_column.length,
                    )?;
                }
            }
            if !passes_fact_comparisons(&row, &fact_comparisons) {
                continue;
            }
            // Relation-local OR predicates can reject fact rows before any
            // hash probes. The final visitor checks the full WHERE filter.
            if fact_row_filter
                .as_ref()
                .is_some_and(|(filters, conditions)| !filters.passes_relation(&row, 0, conditions))
            {
                continue;
            }
            // Reject fact rows before probing dimension hashes or expanding matches.
            let mut passes_fact_exists = true;
            for &filter in &spec.fact_prefiltered_exists {
                if !spec.exists[filter].passes(&row)? {
                    passes_fact_exists = false;
                    break;
                }
            }
            if !passes_fact_exists {
                continue;
            }
            visit(
                0,
                &mut row,
                &stages,
                &mut scratch_keys,
                &spec,
                &mut state,
                output,
            )?;
            if spec.limit.is_some_and(|limit| state.emitted >= limit) {
                return Ok(());
            }
        }
    }
    if scanned != relations[0].rows {
        return Err(invalid("JOIN input row count changed during scan"));
    }
    if !spec.order_by.is_empty() {
        state.ordered_rows.sort_by(|left, right| {
            for (index, descending, nulls_first) in &spec.order_by {
                let ordering = typed::compare_order_cells(
                    &left[*index],
                    &right[*index],
                    *descending,
                    *nulls_first,
                );
                if !ordering.is_eq() {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        });
        for row in state
            .ordered_rows
            .iter()
            .skip(spec.offset)
            .take(spec.limit.unwrap_or(usize::MAX))
        {
            for (position, (index, _, fallback)) in spec.selected.iter().enumerate() {
                if position != 0 {
                    output.write_all(b"\t")?;
                }
                if let Some(expression) =
                    super::scalar_output_index(*index, spec.scalar_projections.len())
                {
                    let value = super::super::multi_aggregate::eval_row_value(
                        &spec.scalar_projections[expression],
                        row,
                    )?;
                    typed::write_value(output, &value)?;
                    continue;
                }
                let source = if matches!(row[*index], typed::Cell::Null) {
                    fallback.unwrap_or(*index)
                } else {
                    *index
                };
                typed::write_typed_value(output, &row[source], spec.types[source])?;
            }
            output.write_all(b"\n")?;
        }
        return Ok(());
    }
    if let Some(formula) = spec.formula {
        let value = evaluate_formula(formula, &state.formula_sums)?;
        return write_scalar_sum(query, value, true, output);
    }
    if spec.scalar_sum {
        return write_scalar_sum(query, state.scalar_sum, true, output);
    }
    if !spec.scalar_aggregates.is_empty() {
        return write_scalar_aggregates(query, &state.scalar_aggregates, true, output);
    }
    if let Some(_) = &query.grouped_count {
        let group_columns = if query.grouped_execution_keys.is_empty() {
            query
                .grouped_count
                .as_ref()
                .expect("grouped query has keys")
        } else {
            &query.grouped_execution_keys
        };
        let ordering = if query.order_by.is_empty() {
            (0..group_columns.len())
                .map(|index| (Some(index), None, false, Some(false)))
                .collect::<Vec<_>>()
        } else {
            query
                .order_by
                .iter()
                .map(|(column, descending, nulls)| {
                    let (key_index, metric_order) = if column.is_window_ratio_order() {
                        if spec.window_ratio.is_none() {
                            return Err(invalid("window ratio ORDER BY has no selected ratio"));
                        }
                        (None, Some(GroupMetricOrder::WindowRatio))
                    } else if let Some(index) = column.case_order_index() {
                        if index >= query.grouped_case_counts.len() {
                            return Err(invalid("grouped CASE ORDER BY metric is out of range"));
                        }
                        (None, Some(GroupMetricOrder::Case(index)))
                    } else if let Some(index) = column.avg_order_index() {
                        if index >= query.grouped_avg_metrics.len() {
                            return Err(invalid("grouped AVG ORDER BY metric is out of range"));
                        }
                        (None, Some(GroupMetricOrder::Average(index)))
                    } else if let Some(index) = column.grouping_order_index() {
                        if !query.grouped_rollup || index >= group_columns.len() {
                            return Err(invalid("GROUPING ORDER BY requires a rollup key"));
                        }
                        (None, Some(GroupMetricOrder::Grouping(index)))
                    } else if column.is_count_order() {
                        (None, None)
                    } else {
                        (
                            Some(
                                group_columns
                                    .iter()
                                    .position(|group| group == column)
                                    .ok_or_else(|| {
                                        invalid("grouped JOIN ORDER BY must use a key or count")
                                    })?,
                            ),
                            None,
                        )
                    };
                    Ok((key_index, metric_order, *descending, *nulls))
                })
                .collect::<Result<Vec<_>, ExecutionError>>()?
        };
        let mut rows = state.groups.into_values().collect::<Vec<_>>();
        if query.grouped_rollup {
            let mut rollups = HashMap::<(usize, Vec<typed::Key>), GroupRow>::new();
            let mut rollup_bytes = 0usize;
            for row in &rows {
                let averages = row
                    .5
                    .as_ref()
                    .ok_or_else(|| invalid("JOIN ROLLUP requires grouped AVG state"))?;
                for prefix in 0..group_columns.len() {
                    let key = row
                        .0
                        .iter()
                        .take(prefix)
                        .map(typed::Cell::key)
                        .collect::<Vec<_>>();
                    if !rollups.contains_key(&(prefix, key.clone())) {
                        if rollups.len() >= MAX_STREAM_GROUPS {
                            return Err(invalid(
                                "chained JOIN ROLLUP supports at most 100000 groups",
                            ));
                        }
                        let variable =
                            row.0.iter().take(prefix).try_fold(0usize, |total, value| {
                                let bytes = match value {
                                    typed::Cell::Text(value) => value.len(),
                                    typed::Cell::Uuid(value) => value.len(),
                                    _ => 0,
                                };
                                total
                                    .checked_add(bytes.saturating_mul(2))
                                    .ok_or_else(|| invalid("JOIN ROLLUP key size overflow"))
                            })?;
                        let fixed = std::mem::size_of::<GroupRow>()
                            + std::mem::size_of::<(usize, Vec<typed::Key>)>()
                            + 64
                            + group_columns.len() * std::mem::size_of::<typed::Cell>()
                            + averages.len()
                                * std::mem::size_of::<(Option<super::super::Number>, u64)>();
                        rollup_bytes = rollup_bytes
                            .checked_add(variable)
                            .and_then(|bytes| bytes.checked_add(fixed))
                            .filter(|&bytes| bytes <= MAX_STREAM_GROUP_BYTES)
                            .ok_or_else(|| invalid("JOIN ROLLUP memory budget exceeded"))?;
                        let mut values = row.0.iter().take(prefix).cloned().collect::<Vec<_>>();
                        values.resize(group_columns.len(), typed::Cell::Null);
                        rollups.insert(
                            (prefix, key.clone()),
                            (
                                values,
                                0,
                                None,
                                None,
                                None,
                                Some(vec![(None, 0); averages.len()].into_boxed_slice()),
                                None,
                                Some(super::super::Number::Unsigned(prefix as u128)),
                            ),
                        );
                    }
                    let target = rollups
                        .get_mut(&(prefix, key))
                        .expect("rollup group was inserted");
                    target.1 = target
                        .1
                        .checked_add(row.1)
                        .ok_or_else(|| invalid("JOIN ROLLUP count overflow"))?;
                    let target_averages = target.5.as_mut().expect("rollup AVG state");
                    for (target, source) in target_averages.iter_mut().zip(averages.iter()) {
                        if let Some(sum) = source.0 {
                            target.0 = Some(match target.0 {
                                Some(previous) => {
                                    super::super::checked_aggregate_sum(previous, sum)?
                                }
                                None => sum,
                            });
                        }
                        target.1 = target
                            .1
                            .checked_add(source.1)
                            .ok_or_else(|| invalid("JOIN ROLLUP AVG count overflow"))?;
                    }
                }
            }
            rows.extend(rollups.into_values());
        }
        if let Some(formula) = spec.grouped_formula {
            for row in &mut rows {
                row.2 = evaluate_formula(
                    formula,
                    row.4.as_deref().expect("grouped formula sums exist"),
                )?;
            }
        }
        if let Some((partition, multiplier)) = spec.window_ratio {
            let mut partition_sums = HashMap::<typed::Key, super::super::Number>::new();
            for row in &rows {
                let Some(sum) = row.2 else {
                    continue;
                };
                let key = row
                    .0
                    .get(partition)
                    .ok_or_else(|| invalid("grouped window partition key is out of range"))?
                    .key();
                if let Some(previous) = partition_sums.get_mut(&key) {
                    *previous = super::super::checked_aggregate_sum(*previous, sum)?;
                } else {
                    partition_sums.insert(key, sum);
                }
            }
            for row in &mut rows {
                let Some(sum) = row.2 else {
                    continue;
                };
                let key = row
                    .0
                    .get(partition)
                    .ok_or_else(|| invalid("grouped window partition key is out of range"))?
                    .key();
                if let Some(total) = partition_sums.get(&key).copied() {
                    row.7 = Some(super::super::Number::Float(
                        number_as_f64(sum) * multiplier / number_as_f64(total),
                    ));
                }
            }
        }
        if let Some(partition) = &spec.window_average {
            let mut partition_sums = HashMap::<Vec<typed::Key>, (super::super::Number, u64)>::new();
            for row in &rows {
                let Some(sum) = row.2 else {
                    continue;
                };
                let key = partition
                    .iter()
                    .map(|&index| {
                        row.0.get(index).map(typed::Cell::key).ok_or_else(|| {
                            invalid("grouped window AVG partition key is out of range")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some((total, groups)) = partition_sums.get_mut(&key) {
                    *total = super::super::checked_aggregate_sum(*total, sum)?;
                    *groups = groups
                        .checked_add(1)
                        .ok_or_else(|| invalid("grouped window AVG group count overflow"))?;
                } else {
                    partition_sums.insert(key, (sum, 1));
                }
            }
            for row in &mut rows {
                let key = partition
                    .iter()
                    .map(|&index| {
                        row.0.get(index).map(typed::Cell::key).ok_or_else(|| {
                            invalid("grouped window AVG partition key is out of range")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some((total, groups)) = partition_sums.get(&key) {
                    row.7 = Some(super::super::Number::Float(
                        number_as_f64(*total) / *groups as f64,
                    ));
                }
            }
        }
        if let Some(having) = &query.grouped_sum_having {
            use super::super::Number;
            if having.value == "\\N" {
                rows.clear();
            } else {
                let value = if having.value.contains(['.', 'e', 'E']) {
                    Number::Float(
                        having
                            .value
                            .parse()
                            .map_err(|_| invalid("invalid JOIN HAVING number"))?,
                    )
                } else if having.value.starts_with('-') {
                    Number::Signed(
                        having
                            .value
                            .parse()
                            .map_err(|_| invalid("invalid JOIN HAVING number"))?,
                    )
                } else {
                    Number::Unsigned(
                        having
                            .value
                            .parse()
                            .map_err(|_| invalid("invalid JOIN HAVING number"))?,
                    )
                };
                rows.retain(|row| {
                    let Some(order) = row.2.and_then(|sum| {
                        super::super::multi_aggregate::compare_expression_numbers(sum, value)
                    }) else {
                        return false;
                    };
                    match having.operator {
                        crate::Operator::Eq => order.is_eq(),
                        crate::Operator::Ne => order.is_ne(),
                        crate::Operator::Lt => order.is_lt(),
                        crate::Operator::Le => order.is_le(),
                        crate::Operator::Gt => order.is_gt(),
                        crate::Operator::Ge => order.is_ge(),
                        _ => false,
                    }
                });
            }
        }
        if let Some(having) = &query.grouped_count_having {
            use super::super::Number;
            if having.value == "\\N" {
                rows.clear();
            } else {
                let value = if having.value.contains(['.', 'e', 'E']) {
                    Number::Float(
                        having
                            .value
                            .parse()
                            .map_err(|_| invalid("invalid JOIN HAVING number"))?,
                    )
                } else if having.value.starts_with('-') {
                    Number::Signed(
                        having
                            .value
                            .parse()
                            .map_err(|_| invalid("invalid JOIN HAVING number"))?,
                    )
                } else {
                    Number::Unsigned(
                        having
                            .value
                            .parse()
                            .map_err(|_| invalid("invalid JOIN HAVING number"))?,
                    )
                };
                rows.retain(|row| {
                    let Some(order) = super::super::multi_aggregate::compare_expression_numbers(
                        Number::Unsigned(row.1.into()),
                        value,
                    ) else {
                        return false;
                    };
                    match having.operator {
                        crate::Operator::Eq => order.is_eq(),
                        crate::Operator::Ne => order.is_ne(),
                        crate::Operator::Lt => order.is_lt(),
                        crate::Operator::Le => order.is_le(),
                        crate::Operator::Gt => order.is_gt(),
                        crate::Operator::Ge => order.is_ge(),
                        _ => false,
                    }
                });
            }
        }
        let compare = |left: &GroupRow, right: &GroupRow| {
            for &(index, metric_order, descending, nulls_first) in &ordering {
                let result = if let Some(index) = index {
                    typed::compare_order_cells(
                        &left.0[index],
                        &right.0[index],
                        descending,
                        nulls_first,
                    )
                } else if let Some(metric_order) = metric_order {
                    let metric_value = |row: &GroupRow| match metric_order {
                        GroupMetricOrder::WindowRatio => row.7,
                        GroupMetricOrder::Case(index) => {
                            row.6.as_deref().and_then(|metrics| metrics[index])
                        }
                        GroupMetricOrder::Average(index) => row
                            .5
                            .as_deref()
                            .and_then(|metrics| metrics.get(index))
                            .and_then(|(sum, count)| {
                                sum.filter(|_| *count > 0).map(|sum| {
                                    super::super::Number::Float(number_as_f64(sum) / *count as f64)
                                })
                            }),
                        GroupMetricOrder::Grouping(index) => {
                            let prefix = match row.7 {
                                Some(super::super::Number::Unsigned(prefix)) => prefix as usize,
                                _ => group_columns.len(),
                            };
                            Some(super::super::Number::Unsigned(u128::from(index >= prefix)))
                        }
                    };
                    let (left_value, right_value) = (metric_value(left), metric_value(right));
                    compare_sum_values(left_value, right_value, descending, nulls_first)
                } else {
                    if sum.is_some() || spec.grouped_formula.is_some() {
                        compare_sum_values(left.2, right.2, descending, nulls_first)
                    } else {
                        let count_order = left.1.cmp(&right.1);
                        if descending {
                            count_order.reverse()
                        } else {
                            count_order
                        }
                    }
                };
                if !result.is_eq() {
                    return result;
                }
            }
            std::cmp::Ordering::Equal
        };
        let bounded = query
            .limit
            .and_then(|limit| query.offset.checked_add(limit))
            .filter(|&end| end < rows.len())
            .map(|end| super::super::sorted_group_positions(&rows, end, compare));
        write_group_header(query, &selected, output)?;
        let mut write_group = |values: &[typed::Cell],
                               count: u64,
                               sum: Option<super::super::Number>,
                               extra_sums: Option<&[Option<super::super::Number>]>,
                               window_ratio: Option<super::super::Number>,
                               averages: Option<&[(Option<super::super::Number>, u64)]>,
                               case_sums: Option<&[Option<super::super::Number>]>|
         -> Result<(), ExecutionError> {
            if query.grouped_keys_only {
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        output.write_all(b"\t")?;
                    }
                    let source = selected[index].0;
                    typed::write_typed_value(output, value, spec.types[source])?;
                }
                writeln!(output)?;
                return Ok(());
            }
            if !query.grouped_output.is_empty() {
                for (index, item) in query.grouped_output.iter().enumerate() {
                    if index > 0 {
                        output.write_all(b"\t")?;
                    }
                    match item {
                        crate::JoinGroupedOutput::Key(key) => {
                            let source = selected[*key].0;
                            let group_index = spec
                                .group_indices
                                .as_ref()
                                .and_then(|indices| {
                                    indices.iter().position(|index| *index == source)
                                })
                                .unwrap_or(*key);
                            typed::write_typed_value(
                                output,
                                &values[group_index],
                                spec.types[source],
                            )?;
                        }
                        crate::JoinGroupedOutput::Count(_) => write!(output, "{count}")?,
                        crate::JoinGroupedOutput::Sum(_) => {
                            typed::write_value(
                                output,
                                &sum.map_or(typed::Cell::Null, typed::Cell::Numeric),
                            )?;
                        }
                        crate::JoinGroupedOutput::SumExtra(metric, _) => {
                            let value = extra_sums
                                .and_then(|sums| sums.get(*metric))
                                .copied()
                                .flatten();
                            typed::write_value(
                                output,
                                &value.map_or(typed::Cell::Null, typed::Cell::Numeric),
                            )?;
                        }
                        crate::JoinGroupedOutput::WindowRatio(_)
                        | crate::JoinGroupedOutput::WindowAverage(_) => {
                            typed::write_value(
                                output,
                                &window_ratio.map_or(typed::Cell::Null, typed::Cell::Numeric),
                            )?;
                        }
                        crate::JoinGroupedOutput::Min(metric, _) => {
                            let value = if *metric == 0 {
                                sum
                            } else {
                                extra_sums
                                    .and_then(|sums| {
                                        sums.get(query.grouped_sum_extras.len() + metric - 1)
                                    })
                                    .copied()
                                    .flatten()
                            };
                            typed::write_value(
                                output,
                                &value.map_or(typed::Cell::Null, typed::Cell::Numeric),
                            )?;
                        }
                        crate::JoinGroupedOutput::Max(metric, _) => {
                            let value = extra_sums
                                .and_then(|sums| {
                                    sums.get(
                                        query.grouped_sum_extras.len()
                                            + query.grouped_min_metrics.len().saturating_sub(1)
                                            + metric,
                                    )
                                })
                                .copied()
                                .flatten();
                            typed::write_value(
                                output,
                                &value.map_or(typed::Cell::Null, typed::Cell::Numeric),
                            )?;
                        }
                        crate::JoinGroupedOutput::Average(metric, _) => {
                            let value = averages
                                .and_then(|metrics| metrics.get(*metric))
                                .and_then(|(sum, observations)| match (sum, observations) {
                                    (Some(sum), count) if *count > 0 => {
                                        Some(super::super::Number::Float(
                                            number_as_f64(*sum) / *count as f64,
                                        ))
                                    }
                                    _ => None,
                                })
                                .map_or(typed::Cell::Null, typed::Cell::Numeric);
                            typed::write_value(output, &value)?;
                        }
                    }
                }
                writeln!(output)?;
                return Ok(());
            }
            if !query.grouped_case_counts.is_empty() {
                let case_sums = case_sums.expect("grouped CASE states allocated");
                if query.scalar_projections.is_empty() {
                    for (index, value) in values.iter().enumerate() {
                        if index > 0 {
                            output.write_all(b"\t")?;
                        }
                        let source = selected[index].0;
                        typed::write_typed_value(output, value, spec.types[source])?;
                    }
                } else {
                    for (position, projection) in query.projection_order.iter().enumerate() {
                        if position > 0 {
                            output.write_all(b"\t")?;
                        }
                        match projection {
                            crate::JoinProjectionOrder::Column(column) => {
                                let source = selected
                                    .get(position)
                                    .ok_or_else(|| invalid("JOIN projection column is missing"))?
                                    .0;
                                let group_index = spec
                                    .group_indices
                                    .as_ref()
                                    .and_then(|indices| {
                                        indices.iter().position(|index| *index == source)
                                    })
                                    .unwrap_or(*column);
                                let value = values.get(group_index).ok_or_else(|| {
                                    invalid("JOIN projection key is missing from its group")
                                })?;
                                typed::write_typed_value(output, value, spec.types[source])?;
                            }
                            crate::JoinProjectionOrder::Scalar(index) => {
                                let projection = query
                                    .scalar_projections
                                    .get(*index)
                                    .ok_or_else(|| invalid("JOIN scalar projection is missing"))?;
                                let crate::ScalarExpr::TextLiteral(value) = &projection.expression
                                else {
                                    return Err(invalid(
                                        "grouped JOIN CASE supports text literal projections",
                                    ));
                                };
                                typed::write_value(
                                    output,
                                    &typed::Cell::Text(value.as_bytes().to_vec()),
                                )?;
                            }
                        }
                    }
                }
                for value in case_sums {
                    output.write_all(b"\t")?;
                    let value = value.map_or(typed::Cell::Null, typed::Cell::Numeric);
                    typed::write_value(output, &value)?;
                }
                writeln!(output)?;
                return Ok(());
            }
            if !query.grouped_avg_metrics.is_empty() {
                let averages = averages.expect("grouped AVG states allocated");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.write_all(b"\t")?;
                    }
                    let source = selected[index].0;
                    typed::write_typed_value(output, value, spec.types[source])?;
                }
                for &(sum, observations) in averages {
                    output.write_all(b"\t")?;
                    let value = match (sum, observations) {
                        (Some(sum), count) if count > 0 => {
                            let numeric = match sum {
                                super::super::Number::Unsigned(value) => value as f64,
                                super::super::Number::Signed(value) => value as f64,
                                super::super::Number::Float(value) => value,
                            };
                            typed::Cell::Numeric(super::super::Number::Float(
                                numeric / count as f64,
                            ))
                        }
                        _ => typed::Cell::Null,
                    };
                    typed::write_value(output, &value)?;
                }
                writeln!(output)?;
                return Ok(());
            }
            let metric = query.grouped_sum_position.unwrap_or(values.len());
            let has_window_ratio = query.grouped_window_ratio.is_some();
            let extra_count = query.grouped_sum_extras.len();
            let last_column = values.len() + extra_count + if has_window_ratio { 1 } else { 0 };
            for index in 0..=last_column {
                if index > 0 {
                    output.write_all(b"\t")?;
                }
                if index == metric {
                    if spec.sum.is_some() || spec.grouped_formula.is_some() {
                        typed::write_value(
                            output,
                            &sum.map_or(typed::Cell::Null, typed::Cell::Numeric),
                        )?;
                    } else {
                        write!(output, "{count}")?;
                    }
                } else if index > metric && index <= metric + extra_count {
                    let value = extra_sums
                        .and_then(|sums| sums.get(index - metric - 1))
                        .copied()
                        .flatten();
                    typed::write_value(
                        output,
                        &value.map_or(typed::Cell::Null, typed::Cell::Numeric),
                    )?;
                } else if has_window_ratio && index == metric + extra_count + 1 {
                    typed::write_value(
                        output,
                        &window_ratio.map_or(typed::Cell::Null, typed::Cell::Numeric),
                    )?;
                } else {
                    let key = if index < metric {
                        index
                    } else {
                        index - 1 - extra_count - if has_window_ratio { 1 } else { 0 }
                    };
                    let source = selected[key].0;
                    if spec.group_year == Some(key) {
                        typed::write_value(output, &values[key])?;
                    } else {
                        typed::write_typed_value(output, &values[key], spec.types[source])?;
                    }
                }
            }
            writeln!(output)?;
            Ok(())
        };
        if let Some(positions) = bounded {
            for index in positions.into_iter().skip(query.offset) {
                let (values, count, sum, _, grouped_sums, avgs, cases, window_ratio) = &rows[index];
                write_group(
                    values,
                    *count,
                    *sum,
                    grouped_sums.as_deref(),
                    *window_ratio,
                    avgs.as_deref(),
                    cases.as_deref(),
                )?;
            }
        } else {
            rows.sort_by(compare);
            for (values, count, sum, _, grouped_sums, avgs, cases, window_ratio) in rows
                .iter()
                .skip(query.offset)
                .take(query.limit.unwrap_or(usize::MAX))
            {
                write_group(
                    values,
                    *count,
                    *sum,
                    grouped_sums.as_deref(),
                    *window_ratio,
                    avgs.as_deref(),
                    cases.as_deref(),
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "streaming/stream_output_limit_tests.rs"]
mod stream_output_limit_tests;

#[cfg(test)]
#[path = "streaming/streaming_hash_budget_tests.rs"]
mod streaming_hash_budget_tests;

#[cfg(test)]
#[path = "streaming/fact_comparison_tests.rs"]
mod fact_comparison_tests;
