//! Bounded left-deep execution for chained joins over persistent tables.

mod correlated_avg;
mod correlated_min;
mod correlated_sum;
mod derived_group_join;
mod nested_sum;
pub use derived_group_join::{execute_derived_grouped_join, execute_derived_grouped_select};
pub use derived_group_join::{execute_derived_union_join, execute_derived_union_select};
pub use nested_sum::execute_nested_join_sum;
mod exists;
mod inner_engine;
mod reorder;
mod row_filters;
mod streaming;
use inner_engine::execute_multi_join_inner;
use super::{ExecutionError, typed};
use crate::{JoinColumn, JoinProjectionOrder, JoinSelect, JoinStep, JoinType};
use rusthouse_storage::inspect_query_source;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

#[cfg(test)]
use super::join::multi_join_row_key as row_key;

use super::join::{
    MAX_INPUT_ROWS, MAX_RESULT_ROWS, MAX_STREAM_BUILD_BYTES,
    multi_join_row_key_into as row_key_into, read_projected_build_rows,
    read_projected_build_rows_matching,
};

const MAX_CHAIN_BYTES: usize = crate::join::MAX_JOIN_CHAIN_BYTES;
const MAX_CHAIN_COLUMNS: usize = 256;
const MAX_CHAIN_HASH_BYTES: usize = MAX_CHAIN_BYTES;
const SCALAR_OUTPUT_INDEX_BASE: usize = usize::MAX;

type RelationComparison = (usize, crate::Operator, typed::Cell);

fn relation_comparison_filters(
    comparisons: &[(usize, crate::Operator, typed::Cell)],
    relation_start: usize,
    physical_columns: usize,
) -> Vec<RelationComparison> {
    comparisons
        .iter()
        .filter_map(|(index, operator, value)| {
            index
                .checked_sub(relation_start)
                .filter(|local| *local < physical_columns)
                .map(|local| (local, *operator, value.clone()))
        })
        .collect()
}

fn passes_relation_comparison_filters(row: &[typed::Cell], filters: &[RelationComparison]) -> bool {
    filters.iter().all(|(index, operator, value)| {
        row.get(*index)
            .is_some_and(|candidate| typed::matches_comparison(candidate, value, *operator))
    })
}

#[cfg(test)]
mod relation_comparison_filter_tests {
    use super::*;

    #[test]
    fn comparison_filters_are_local_and_all_must_pass() {
        let comparisons = [
            (
                1,
                crate::Operator::Eq,
                typed::Cell::Numeric(super::super::Number::Unsigned(1)),
            ),
            (
                2,
                crate::Operator::Gt,
                typed::Cell::Numeric(super::super::Number::Unsigned(10)),
            ),
            (
                3,
                crate::Operator::Le,
                typed::Cell::Numeric(super::super::Number::Unsigned(20)),
            ),
            (
                4,
                crate::Operator::Eq,
                typed::Cell::Numeric(super::super::Number::Unsigned(4)),
            ),
        ];
        let filters = relation_comparison_filters(&comparisons, 2, 2);

        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].0, 0);
        assert_eq!(filters[1].0, 1);
        assert!(passes_relation_comparison_filters(
            &[
                typed::Cell::Numeric(super::super::Number::Unsigned(11)),
                typed::Cell::Numeric(super::super::Number::Unsigned(20)),
            ],
            &filters,
        ));
        assert!(!passes_relation_comparison_filters(
            &[
                typed::Cell::Numeric(super::super::Number::Unsigned(10)),
                typed::Cell::Numeric(super::super::Number::Unsigned(20)),
            ],
            &filters,
        ));
        assert!(!passes_relation_comparison_filters(
            &[
                typed::Cell::Null,
                typed::Cell::Numeric(super::super::Number::Unsigned(20))
            ],
            &filters,
        ));
    }
}

fn scalar_output_index(index: usize, projection_count: usize) -> Option<usize> {
    SCALAR_OUTPUT_INDEX_BASE
        .checked_sub(index)
        .filter(|scalar_index| *scalar_index < projection_count)
}

/// Hash table keys are owned again after the build-side rows are decoded.
/// Include duplicate row index lists, which are separate from row retention.
fn charge_hash_bytes(used: &mut usize, additional: usize) -> Result<(), ExecutionError> {
    let next = used
        .checked_add(additional)
        .ok_or_else(|| invalid("JOIN hash memory estimate overflow"))?;
    if next > MAX_CHAIN_HASH_BYTES {
        return Err(invalid("JOIN hash keys exceed retained memory budget"));
    }
    *used = next;
    Ok(())
}

fn hash_key_bytes(key: &[typed::Key]) -> Result<usize, ExecutionError> {
    let fixed = key
        .len()
        .checked_mul(std::mem::size_of::<typed::Key>())
        .and_then(|n| n.checked_add(std::mem::size_of::<(Vec<typed::Key>, Bucket)>()))
        .ok_or_else(|| invalid("JOIN hash memory estimate overflow"))?;
    key.iter().try_fold(fixed, |bytes, value| {
        let payload = match value {
            typed::Key::Text(value) => value.len(),
            typed::Key::Uuid(value) => value.len(),
            _ => 0,
        };
        bytes
            .checked_add(payload)
            .ok_or_else(|| invalid("JOIN hash memory estimate overflow"))
    })
}

fn write_join_header<W: Write>(
    output: &mut W,
    selected: &[(usize, String, Option<usize>)],
) -> Result<(), ExecutionError> {
    for (index, (_, name, _)) in selected.iter().enumerate() {
        if index != 0 {
            output.write_all(b"\t")?;
        }
        output.write_all(name.as_bytes())?;
    }
    output.write_all(b"\n")?;
    Ok(())
}

fn write_scalar_sum<W: Write>(
    query: &JoinSelect,
    value: Option<super::Number>,
    write_row: bool,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let name = query.aggregate_alias.clone().unwrap_or_else(|| {
        format!(
            "sum({})",
            crate::multi_aggregate::render_expr(query.scalar_sum.as_ref().expect("scalar SUM"))
        )
    });
    writeln!(output, "{name}")?;
    if write_row {
        if let (true, Some(super::Number::Float(float))) = (query.scalar_result_typed, value) {
            write!(output, "{float:?}")?;
        } else {
            typed::write_value(
                output,
                &value.map_or(typed::Cell::Null, typed::Cell::Numeric),
            )?;
        }
        writeln!(output)?;
    }
    Ok(())
}

fn write_scalar_aggregate_header<W: Write>(
    query: &JoinSelect,
    output: &mut W,
) -> Result<(), ExecutionError> {
    for (index, aggregate) in query.scalar_aggregates.iter().enumerate() {
        if index > 0 {
            output.write_all(b"\t")?;
        }
        output.write_all(aggregate.alias.as_bytes())?;
    }
    output.write_all(b"\n")?;
    Ok(())
}

fn write_group_header<W: Write>(
    query: &JoinSelect,
    selected: &[(usize, String, Option<usize>)],
    output: &mut W,
) -> Result<(), ExecutionError> {
    if !query.grouped_output.is_empty() {
        for (index, item) in query.grouped_output.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\t")?;
            }
            let name = match item {
                crate::JoinGroupedOutput::Key(key) => &selected[*key].1,
                crate::JoinGroupedOutput::Count(alias)
                | crate::JoinGroupedOutput::Sum(alias)
                | crate::JoinGroupedOutput::WindowRatio(alias)
                | crate::JoinGroupedOutput::WindowAverage(alias) => alias,
                crate::JoinGroupedOutput::SumExtra(_, alias) => alias,
                crate::JoinGroupedOutput::Min(_, alias)
                | crate::JoinGroupedOutput::Max(_, alias)
                | crate::JoinGroupedOutput::Average(_, alias) => alias,
            };
            output.write_all(name.as_bytes())?;
        }
        output.write_all(b"\n")?;
        return Ok(());
    }
    if query.grouped_keys_only {
        return write_join_header(output, selected);
    }
    if !query.grouped_avg_metrics.is_empty() {
        for (_, name, _) in selected {
            output.write_all(name.as_bytes())?;
            output.write_all(b"\t")?;
        }
        for (index, (_, name)) in query.grouped_avg_metrics.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\t")?;
            }
            output.write_all(name.as_bytes())?;
        }
        output.write_all(b"\n")?;
        return Ok(());
    }
    if !query.grouped_case_counts.is_empty() {
        for (index, (_, name, _)) in selected.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\t")?;
            }
            output.write_all(name.as_bytes())?;
        }
        for (_, _, alias) in &query.grouped_case_counts {
            output.write_all(b"\t")?;
            output.write_all(alias.as_bytes())?;
        }
        output.write_all(b"\n")?;
        return Ok(());
    }
    let metric = query.grouped_sum_position.unwrap_or(selected.len());
    let has_window_ratio = query.grouped_window_ratio.is_some();
    let extra_sums = query.grouped_sum_extras.len();
    let last_column = selected.len() + extra_sums + if has_window_ratio { 1 } else { 0 };
    for index in 0..=last_column {
        if index > 0 {
            output.write_all(b"\t")?;
        }
        if index == metric {
            output.write_all(
                query
                    .aggregate_alias
                    .as_deref()
                    .unwrap_or("count")
                    .as_bytes(),
            )?;
        } else if index > metric && index <= metric + extra_sums {
            output.write_all(query.grouped_sum_extras[index - metric - 1].1.as_bytes())?;
        } else if has_window_ratio && index == metric + extra_sums + 1 {
            output.write_all(
                query
                    .grouped_window_ratio
                    .as_ref()
                    .expect("window ratio checked")
                    .alias
                    .as_bytes(),
            )?;
        } else {
            let key = if index < metric {
                index
            } else {
                index - 1 - extra_sums - if has_window_ratio { 1 } else { 0 }
            };
            output.write_all(selected[key].1.as_bytes())?;
        }
    }
    output.write_all(b"\n")?;
    Ok(())
}

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

/// Keep the common unique-key join bucket inline instead of allocating
/// a heap vector for every dimension key.
enum Bucket {
    One(usize),
    Many(Vec<usize>),
}

impl Bucket {
    fn push(&mut self, index: usize) {
        match self {
            Self::One(first) => *self = Self::Many(vec![*first, index]),
            Self::Many(indices) => indices.push(index),
        }
    }

    fn iter(&self) -> std::slice::Iter<'_, usize> {
        match self {
            Self::One(index) => std::slice::from_ref(index).iter(),
            Self::Many(indices) => indices.iter(),
        }
    }
}

fn validate_sum_expression(
    expression: &super::multi_aggregate::BoundExpr,
    relations: &[Relation],
) -> Result<(), ExecutionError> {
    use super::multi_aggregate::BoundExpr;
    let check = |index: usize, text: bool| {
        let kind = typed::base_type(column_type(relations, index));
        let valid = if text {
            kind == "String"
        } else {
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
            )
        };
        if valid {
            Ok(())
        } else {
            Err(invalid(if text {
                "SUM text condition requires a String column"
            } else {
                "SUM expression requires numeric columns"
            }))
        }
    };
    match expression {
        BoundExpr::Column(index) | BoundExpr::AffineColumn { index, .. } => check(*index, false),
        BoundExpr::TextLength(index)
        | BoundExpr::TextCompare { index, .. }
        | BoundExpr::TextLike { index, .. } => check(*index, true),
        BoundExpr::TextLiteral(_) | BoundExpr::TextConcat(_) | BoundExpr::TextSubstring { .. } => {
            Err(invalid("SUM expression cannot use a text projection"))
        }
        BoundExpr::Round { value, .. } => validate_sum_expression(value, relations),
        BoundExpr::IsNull { .. } => Ok(()),
        BoundExpr::Integer(_) | BoundExpr::Float(_) => Ok(()),
        BoundExpr::Add(a, b)
        | BoundExpr::Subtract(a, b)
        | BoundExpr::Multiply(a, b)
        | BoundExpr::Divide(a, b)
        | BoundExpr::Modulo(a, b)
        | BoundExpr::Compare(a, _, b)
        | BoundExpr::And(a, b)
        | BoundExpr::Or(a, b) => {
            validate_sum_expression(a, relations)?;
            validate_sum_expression(b, relations)
        }
        BoundExpr::Case {
            branches,
            else_expr,
        } => {
            for (condition, value) in branches {
                validate_sum_expression(condition, relations)?;
                validate_sum_expression(value, relations)?;
            }
            if let Some(value) = else_expr {
                validate_sum_expression(value, relations)?;
            }
            Ok(())
        }
    }
}

struct BoundFormula {
    sums: Vec<BoundSum>,
    expression: super::multi_aggregate::BoundExpr,
}

struct BoundSum {
    expression: super::multi_aggregate::BoundExpr,
    indices: Vec<usize>,
}

struct Relation {
    alias: String,
    names: Vec<String>,
    types: Vec<String>,
    physical_columns: usize,
    virtual_columns: Vec<BoundVirtualColumn>,
    start: usize,
    rows: u64,
}

struct BoundVirtualColumn {
    index: usize,
    source_index: usize,
    start: usize,
    length: usize,
}

type BoundSubstringComparison = (usize, usize, usize, crate::Operator, usize, usize, usize);

fn column_index(relations: &[Relation], column: &JoinColumn) -> Result<usize, ExecutionError> {
    if column.relation.is_empty() {
        let mut found = None;
        for relation in relations {
            if let Some(index) = relation.names.iter().position(|name| name == &column.name) {
                if found.replace(relation.start + index).is_some() {
                    return Err(invalid(format!("ambiguous JOIN column: {}", column.name)));
                }
            }
        }
        return found.ok_or_else(|| invalid(format!("unknown JOIN column: {}", column.name)));
    }
    let relation = relations
        .iter()
        .find(|relation| relation.alias == column.relation)
        .ok_or_else(|| invalid(format!("unknown JOIN qualifier: {}", column.relation)))?;
    relation
        .names
        .iter()
        .position(|name| name == &column.name)
        .map(|index| relation.start + index)
        .ok_or_else(|| {
            invalid(format!(
                "unknown JOIN column: {}.{}",
                column.relation, column.name
            ))
        })
}

fn column_type(relations: &[Relation], index: usize) -> &str {
    let relation = relations
        .iter()
        .rev()
        .find(|relation| index >= relation.start)
        .expect("column index validated");
    &relation.types[index - relation.start]
}

fn retained_row_bytes(row: &[typed::Cell]) -> Result<usize, ExecutionError> {
    let base = row
        .len()
        .checked_mul(std::mem::size_of::<typed::Cell>())
        .and_then(|value| value.checked_add(std::mem::size_of::<Vec<typed::Cell>>()))
        .ok_or_else(|| invalid("JOIN result size overflow"))?;
    row.iter().try_fold(base, |total, cell| {
        let additional = match cell {
            typed::Cell::Text(value) => value.len(),
            typed::Cell::Uuid(value) => value.len(),
            _ => 0,
        };
        total
            .checked_add(additional)
            .ok_or_else(|| invalid("JOIN result size overflow"))
    })
}

fn push_row(
    rows: &mut Vec<Vec<typed::Cell>>,
    row: Vec<typed::Cell>,
    retained: &mut usize,
) -> Result<(), ExecutionError> {
    if rows.len() >= MAX_RESULT_ROWS {
        return Err(invalid("JOIN produces more than 131072 rows"));
    }
    *retained = retained
        .checked_add(retained_row_bytes(&row)?)
        .ok_or_else(|| invalid("JOIN result size overflow"))?;
    if *retained > MAX_CHAIN_BYTES {
        return Err(invalid("JOIN exceeds its retained-row memory budget"));
    }
    rows.push(row);
    Ok(())
}

/// Drop filtered final-stage rows before charging the bounded intermediate.
/// An anti-join can otherwise reject a large valid result before WHERE runs.
fn push_visible_row(
    rows: &mut Vec<Vec<typed::Cell>>,
    row: Vec<typed::Cell>,
    retained: &mut usize,
    filter: Option<(usize, bool)>,
) -> Result<(), ExecutionError> {
    if filter.is_some_and(|(index, not_null)| matches!(row[index], typed::Cell::Null) == not_null) {
        return Ok(());
    }
    push_row(rows, row, retained)
}

/// Resolve USING only when exactly one existing input supplies that key.
/// Ambiguous names must use qualified ON instead.
fn using_column_pair(relations: &[Relation], name: &str) -> Result<(usize, usize), ExecutionError> {
    let (right, earlier) = relations
        .split_last()
        .ok_or_else(|| invalid("JOIN USING has no relations"))?;
    let mut previous = None;
    for relation in earlier {
        if let Some(index) = relation.names.iter().position(|column| column == name) {
            if previous.replace(relation.start + index).is_some() {
                return Err(invalid(format!("ambiguous JOIN USING column: {name}")));
            }
        }
    }
    let left = previous.ok_or_else(|| invalid(format!("unknown JOIN USING column: {name}")))?;
    let right_index = right
        .names
        .iter()
        .position(|column| column == name)
        .ok_or_else(|| invalid(format!("unknown JOIN USING column: {name}")))?;
    Ok((left, right.start + right_index))
}

fn step_keys(
    step: &JoinStep,
    relations: &[Relation],
    left_width: usize,
) -> Result<(Vec<usize>, Vec<usize>), ExecutionError> {
    let count = step.keys.len() + step.using_keys.len();
    let mut left_keys = Vec::with_capacity(count);
    let mut right_keys = Vec::with_capacity(count);
    for name in &step.using_keys {
        let (left, right) = using_column_pair(relations, name)?;
        let left_type = column_type(relations, left);
        let right_type = column_type(relations, right);
        if !super::join::compatible_join_types(left_type, right_type) {
            return Err(invalid(format!(
                "JOIN key types differ: {left_type} and {right_type}"
            )));
        }
        left_keys.push(left);
        right_keys.push(right - left_width);
    }
    for (first, second) in step.keys.iter().take(if step.using_keys.is_empty() {
        usize::MAX
    } else {
        0
    }) {
        let first_index = column_index(relations, first)?;
        let second_index = column_index(relations, second)?;
        let (left, right) = if first_index < left_width && second_index >= left_width {
            (first_index, second_index)
        } else if second_index < left_width && first_index >= left_width {
            (second_index, first_index)
        } else {
            return Err(invalid(
                "JOIN ON must compare one existing relation to the newly joined table",
            ));
        };
        let left_type = column_type(relations, left);
        let right_type = column_type(relations, right);
        if !super::join::compatible_join_types(left_type, right_type) {
            return Err(invalid(format!(
                "JOIN key types differ: {left_type} and {right_type}"
            )));
        }
        left_keys.push(left);
        right_keys.push(right - left_width);
    }
    Ok((left_keys, right_keys))
}

/// Bind the output schema without opening or scanning any table rows.
fn output_columns(
    query: &JoinSelect,
    relations: &[Relation],
    initial_step: &JoinStep,
) -> Result<Vec<(usize, String, Option<usize>)>, ExecutionError> {
    // For unqualified *, the USING key appears once. With a RIGHT/FULL
    // join, fall back to the newly joined value when the old side is NULL.
    let mut omitted = HashMap::new();
    for (index, step) in std::iter::once(initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        for name in &step.using_keys {
            let (left, right) = using_column_pair(&relations[..=index + 1], name)?;
            omitted.insert(right, left);
        }
    }
    let mut selected: Vec<(usize, String, Option<usize>)> = Vec::new();
    if let Some(columns) = &query.columns {
        for (position, column) in columns.iter().enumerate() {
            if column.name == "*" {
                let relation = relations
                    .iter()
                    .find(|relation| relation.alias == column.relation)
                    .ok_or_else(|| {
                        invalid(format!("unknown JOIN qualifier: {}", column.relation))
                    })?;
                for (index, name) in relation.names.iter().enumerate() {
                    selected.push((
                        relation.start + index,
                        format!("{}.{}", relation.alias, name),
                        None,
                    ));
                }
            } else {
                selected.push((
                    column_index(relations, column)?,
                    if let Some((_, alias)) = query
                        .grouped_year
                        .as_ref()
                        .filter(|(index, _)| *index == position)
                    {
                        alias.clone()
                    } else if let Some((_, alias)) = query
                        .grouped_key_aliases
                        .iter()
                        .find(|(index, _)| *index == position)
                    {
                        alias.clone()
                    } else if column.relation.is_empty() {
                        column.name.clone()
                    } else {
                        column.name.clone()
                    },
                    None,
                ));
            }
        }
    } else {
        for relation in relations {
            for (index, name) in relation.names.iter().enumerate() {
                let absolute = relation.start + index;
                if let Some(&left) = omitted.get(&absolute) {
                    let existing = selected
                        .iter_mut()
                        .find(|(index, _, _)| *index == left)
                        .ok_or_else(|| invalid("JOIN USING output key was not projected"))?;
                    existing.2 = Some(absolute);
                } else {
                    selected.push((absolute, format!("{}.{}", relation.alias, name), None));
                }
            }
        }
    }
    if !query.scalar_projections.is_empty() {
        let mut ordered = Vec::with_capacity(query.projection_order.len());
        for projection in &query.projection_order {
            match projection {
                JoinProjectionOrder::Column(index) => ordered.push(
                    selected
                        .get(*index)
                        .cloned()
                        .ok_or_else(|| invalid("JOIN projection column is out of range"))?,
                ),
                JoinProjectionOrder::Scalar(index) => {
                    let projection = query
                        .scalar_projections
                        .get(*index)
                        .ok_or_else(|| invalid("JOIN scalar projection is out of range"))?;
                    ordered.push((
                        SCALAR_OUTPUT_INDEX_BASE - *index,
                        projection.alias.clone(),
                        None,
                    ));
                }
            }
        }
        return Ok(ordered);
    }
    Ok(selected)
}

fn bind_scalar_projections(
    query: &JoinSelect,
    relations: &[Relation],
) -> Result<Vec<super::multi_aggregate::BoundExpr>, ExecutionError> {
    let mut indices = HashMap::new();
    let mut unqualified: HashMap<String, Option<usize>> = HashMap::new();
    for relation in relations {
        for (column_index, name) in relation.names.iter().enumerate() {
            let index = relation.start + column_index;
            indices.insert(format!("{}\0{name}", relation.alias), index);
            unqualified
                .entry(name.clone())
                .and_modify(|existing| *existing = None)
                .or_insert(Some(index));
        }
    }
    for (name, index) in unqualified {
        if let Some(index) = index {
            indices.insert(name, index);
        }
    }
    query
        .scalar_projections
        .iter()
        .map(|projection| super::multi_aggregate::BoundExpr::bind(&projection.expression, &indices))
        .collect()
}

fn write_global_case_aggregates<W: Write>(
    query: &JoinSelect,
    rows: &[Vec<typed::Cell>],
    relations: &[Relation],
    output: &mut W,
) -> Result<bool, ExecutionError> {
    if !query.grouped_count.as_ref().is_some_and(Vec::is_empty)
        || query.grouped_case_counts.is_empty()
    {
        return Ok(false);
    }

    let mut indices = HashMap::new();
    for relation in relations {
        for (column_index, name) in relation.names.iter().enumerate() {
            indices.insert(
                format!("{}\0{name}", relation.alias),
                relation.start + column_index,
            );
            indices.insert(name.clone(), relation.start + column_index);
        }
    }
    let mut filters = Vec::with_capacity(query.grouped_case_counts.len());
    let mut values = Vec::with_capacity(query.grouped_case_counts.len());
    for (filter, value, _) in &query.grouped_case_counts {
        let crate::GroupedFilter::CaseExpression { condition, .. } = filter else {
            return Err(invalid(
                "global grouped CASE aggregates require expression conditions",
            ));
        };
        filters.push(super::multi_aggregate::BoundExpr::bind(
            condition, &indices,
        )?);
        values.push(match value {
            crate::GroupedCaseValue::Count => None,
            crate::GroupedCaseValue::Column(column) => {
                let name = if column.relation.is_empty() {
                    column.name.clone()
                } else {
                    format!("{}\0{}", column.relation, column.name)
                };
                Some(super::multi_aggregate::BoundExpr::bind(
                    &crate::ScalarExpr::Column(name),
                    &indices,
                )?)
            }
            crate::GroupedCaseValue::Expression(expression) => Some(
                super::multi_aggregate::BoundExpr::bind(expression, &indices)?,
            ),
        });
    }

    for (index, (_, _, alias)) in query.grouped_case_counts.iter().enumerate() {
        if index != 0 {
            output.write_all(b"\t")?;
        }
        output.write_all(alias.as_bytes())?;
    }
    output.write_all(b"\n")?;
    if query.offset > 0 || query.limit == Some(0) {
        return Ok(true);
    }

    let mut sums = vec![None; filters.len()];
    for row in rows {
        for index in 0..filters.len() {
            let matches = match super::multi_aggregate::eval_row_arithmetic(&filters[index], row)? {
                Some(super::Number::Unsigned(value)) => value != 0,
                Some(super::Number::Signed(value)) => value != 0,
                Some(super::Number::Float(value)) => value != 0.0,
                None => false,
            };
            if matches {
                let value = match &values[index] {
                    Some(expression) => {
                        super::multi_aggregate::eval_row_arithmetic(expression, row)?
                    }
                    None => Some(super::Number::Unsigned(1)),
                };
                if let Some(value) = value {
                    sums[index] = Some(match sums[index] {
                        Some(current) => super::checked_aggregate_sum(current, value)?,
                        None => value,
                    });
                }
            }
        }
    }
    if rows.is_empty() {
        for (index, _) in sums.iter().enumerate() {
            if index != 0 {
                output.write_all(b"\t")?;
            }
            typed::write_value(output, &typed::Cell::Null)?;
        }
    } else {
        for (index, sum) in sums.into_iter().enumerate() {
            if index != 0 {
                output.write_all(b"\t")?;
            }
            typed::write_value(output, &sum.map_or(typed::Cell::Null, typed::Cell::Numeric))?;
        }
    }
    output.write_all(b"\n")?;
    Ok(true)
}

/// Retain only physical streams needed by the query's output, join keys,
/// optional NULL filter, and ORDER BY. Output rows still carry full-width
/// logical slots so existing join-key and projection indices remain valid.
fn required_physical_columns(
    query: &JoinSelect,
    relations: &[Relation],
    initial_step: &JoinStep,
    row_filters: Option<&row_filters::RowFilters>,
    avg_filter: Option<&correlated_avg::BoundAvg>,
    min_filter: Option<&correlated_min::BoundMin>,
    sum_filter: Option<&correlated_sum::BoundSum>,
) -> Result<Vec<Vec<usize>>, ExecutionError> {
    let mut required = relations
        .iter()
        .map(|relation| vec![false; relation.names.len()])
        .collect::<Vec<_>>();
    let mut retain_absolute = |absolute: usize| {
        if let Some((index, relation)) = relations
            .iter()
            .enumerate()
            .rev()
            .find(|(_, relation)| absolute >= relation.start)
        {
            required[index][absolute - relation.start] = true;
        }
    };
    if query.aggregate.is_none() {
        if let Some(crate::ScalarExpr::Column(name)) = &query.scalar_sum {
            let (relation, column) = if let Some(relation) = &query.scalar_sum_qualifier {
                (relation.as_str(), name.as_str())
            } else {
                ("", name.as_str())
            };
            retain_absolute(column_index(
                relations,
                &JoinColumn {
                    relation: relation.to_owned(),
                    name: column.to_owned(),
                },
            )?);
        } else if let Some(formula) = &query.scalar_formula {
            for expression in &formula.sums {
                let mut names = Vec::new();
                expression.referenced_columns(&mut names);
                for name in names {
                    let (relation, column) = name
                        .split_once('\0')
                        .map_or(("", name.as_str()), |(relation, column)| (relation, column));
                    retain_absolute(column_index(
                        relations,
                        &JoinColumn {
                            relation: relation.to_owned(),
                            name: column.to_owned(),
                        },
                    )?);
                }
            }
        } else if !query.scalar_aggregates.is_empty() {
            for item in &query.scalar_aggregates {
                if let crate::JoinAggregate::CountColumn(column)
                | crate::JoinAggregate::Numeric { column, .. } = &item.aggregate
                {
                    retain_absolute(column_index(relations, column)?);
                }
            }
        } else if let Some(columns) = &query.columns {
            for column in columns {
                if column.name == "*" {
                    let relation = relations
                        .iter()
                        .find(|relation| relation.alias == column.relation)
                        .ok_or_else(|| {
                            invalid(format!("unknown JOIN qualifier: {}", column.relation))
                        })?;
                    for local in 0..relation.names.len() {
                        retain_absolute(relation.start + local);
                    }
                } else {
                    retain_absolute(column_index(relations, column)?);
                }
            }
        } else {
            for relation in relations {
                for local in 0..relation.names.len() {
                    retain_absolute(relation.start + local);
                }
            }
        }
    }
    for projection in &query.scalar_projections {
        let mut names = Vec::new();
        projection.expression.referenced_columns(&mut names);
        for name in names {
            let (relation, column) = name
                .split_once('\0')
                .map_or(("", name.as_str()), |(relation, column)| (relation, column));
            retain_absolute(column_index(
                relations,
                &JoinColumn {
                    relation: relation.to_owned(),
                    name: column.to_owned(),
                },
            )?);
        }
    }
    for (filter, value, _) in &query.grouped_case_counts {
        if let Some(column) = filter.column() {
            let (relation, name) = column
                .split_once('\0')
                .map_or(("", column), |(relation, name)| (relation, name));
            retain_absolute(column_index(
                relations,
                &JoinColumn {
                    relation: relation.to_owned(),
                    name: name.to_owned(),
                },
            )?);
        }
        if let crate::GroupedFilter::CaseExpression { condition, .. } = filter {
            let mut names = Vec::new();
            condition.referenced_columns(&mut names);
            for name in names {
                let (relation, column) = name
                    .split_once('\0')
                    .map_or(("", name.as_str()), |(relation, column)| (relation, column));
                retain_absolute(column_index(
                    relations,
                    &JoinColumn {
                        relation: relation.to_owned(),
                        name: column.to_owned(),
                    },
                )?);
            }
        }
        match value {
            crate::GroupedCaseValue::Count => {}
            crate::GroupedCaseValue::Column(column) => {
                retain_absolute(column_index(relations, column)?);
            }
            crate::GroupedCaseValue::Expression(expression) => {
                let mut names = Vec::new();
                expression.referenced_columns(&mut names);
                for name in names {
                    let (relation, column) = name
                        .split_once('\0')
                        .map_or(("", name.as_str()), |(relation, column)| (relation, column));
                    retain_absolute(column_index(
                        relations,
                        &JoinColumn {
                            relation: relation.to_owned(),
                            name: column.to_owned(),
                        },
                    )?);
                }
            }
        }
    }
    if let Some(crate::JoinAggregate::CountColumn(column)) = &query.aggregate {
        retain_absolute(column_index(relations, column)?);
    }
    if let Some((column, _)) = &query.where_null {
        retain_absolute(column_index(relations, column)?);
    }
    for (column, _, _) in &query.order_by {
        if !(column.is_count_order()
            && (query.aggregate == Some(crate::JoinAggregate::CountStar)
                || !query.scalar_aggregates.is_empty()
                || query.scalar_sum.is_some()))
            && column.scalar_order_index().is_none()
        {
            retain_absolute(column_index(relations, column)?);
        }
    }
    for (column, _, _) in query
        .where_comparison
        .iter()
        .chain(&query.where_comparisons_extra)
    {
        retain_absolute(column_index(relations, column)?);
    }
    for (left, _, right) in &query.where_column_comparisons {
        retain_absolute(column_index(relations, left)?);
        retain_absolute(column_index(relations, right)?);
    }
    if let Some(filters) = row_filters {
        for &index in &filters.indices {
            retain_absolute(index);
        }
    }
    if let Some(filter) = avg_filter {
        retain_absolute(filter.outer_key);
        retain_absolute(filter.outer_value);
    }
    if let Some(filter) = min_filter {
        retain_absolute(filter.outer_key);
        retain_absolute(filter.outer_value);
    }
    if let Some(filter) = sum_filter {
        for index in filter.outer_keys {
            retain_absolute(index);
        }
        retain_absolute(filter.outer_value);
    }
    for (step_index, step) in std::iter::once(initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let width = relations[step_index + 1].start;
        let (left, right) = step_keys(step, &relations[..=step_index + 1], width)?;
        for key in left {
            retain_absolute(key);
        }
        for key in right {
            retain_absolute(width + key);
        }
    }
    // A CROSS JOIN or a COUNT-only relation can have no referenced values.
    // Keep one column to obtain exact row counts from the typed reader.
    let mut indices = Vec::with_capacity(relations.len());
    for (relation, columns) in relations.iter().zip(required) {
        let mut keep = columns
            .into_iter()
            .enumerate()
            .filter_map(|(index, needed)| needed.then_some(index))
            .collect::<Vec<_>>();
        if keep.is_empty() {
            if relation.names.is_empty() {
                return Err(invalid("JOIN requires nonempty table schemas"));
            }
            keep.push(0);
        }
        indices.push(keep);
    }
    Ok(indices)
}

const MAX_IN_SUBQUERY_BYTES: usize = 16 * 1024 * 1024;
const MAX_IN_SUBQUERY_KEYS: usize = 100_000;

struct SubqueryOutput(Vec<u8>);
impl Write for SubqueryOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_IN_SUBQUERY_BYTES {
            return Err(std::io::Error::other("JOIN IN subquery exceeds 16 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Evaluate an uncorrelated HAVING scalar while the caller holds the database read lock.
pub fn bind_join_having_subquery(
    query: &mut JoinSelect,
    paths: &[PathBuf],
) -> Result<(), ExecutionError> {
    let having = query
        .grouped_sum_having
        .as_mut()
        .ok_or_else(|| invalid("missing HAVING"))?;
    let inner = having
        .subquery
        .as_ref()
        .ok_or_else(|| invalid("missing HAVING subquery"))?;
    if (inner.scalar_sum.is_none() && inner.scalar_formula.is_none())
        || inner.grouped_count.is_some()
        || inner.limit.is_some()
        || inner.offset != 0
        || inner.in_subquery.is_some()
        || inner.grouped_sum_having.is_some()
    {
        return Err(invalid("HAVING subquery requires an ungrouped scalar SUM"));
    }
    let mut output = SubqueryOutput(Vec::new());
    let mut inner = (**inner).clone();
    inner.scalar_result_typed = true;
    execute_multi_join(&inner, paths, &mut output)?;
    let text =
        std::str::from_utf8(&output.0).map_err(|_| invalid("invalid HAVING scalar output"))?;
    let mut lines = text.lines();
    let _header = lines
        .next()
        .ok_or_else(|| invalid("missing HAVING scalar header"))?;
    let value = lines
        .next()
        .ok_or_else(|| invalid("missing HAVING scalar value"))?;
    if lines.next().is_some() || value.contains('\t') {
        return Err(invalid("HAVING subquery must return one scalar"));
    }
    if value != "\\N" {
        let valid = if value.contains(['.', 'e', 'E']) {
            value.parse::<f64>().is_ok_and(f64::is_finite)
        } else if value.starts_with('-') {
            value.parse::<i128>().is_ok()
        } else {
            value.parse::<u128>().is_ok()
        };
        if !valid {
            return Err(invalid("invalid HAVING scalar value"));
        }
    }
    having.value = value.to_owned();
    having.subquery = None;
    Ok(())
}

/// Bind one supported uncorrelated IN subquery while callers hold a database read lock.
pub fn bind_join_in_subquery(
    query: &mut JoinSelect,
    inner_path: &std::path::Path,
    outer_paths: &[PathBuf],
) -> Result<(), ExecutionError> {
    let subquery = query
        .in_subquery
        .as_ref()
        .ok_or_else(|| invalid("JOIN IN subquery is missing"))?;
    let supported = if subquery.negated {
        query.grouped_distinct.is_some()
            && query.grouped_sum.is_none()
            && query
                .scalar_where
                .as_ref()
                .is_some_and(|branches| branches.len() <= 1)
    } else {
        query.grouped_sum.is_some()
            && query.grouped_distinct.is_none()
            && !query.steps.is_empty()
            && query.scalar_where.is_none()
    };
    if !supported
        || query.grouped_count.is_none()
        || query.aggregate.is_some()
        || query.scalar_sum.is_some()
        || query.scalar_formula.is_some()
        || !query.grouped_case_counts.is_empty()
        || !query.where_in_filters.is_empty()
        || !query.where_like_filters.is_empty()
        || query.kind != JoinType::Inner
        || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        || query.where_null.is_some()
        || !subquery.column.relation.is_empty()
        || outer_paths.len() != query.steps.len() + 2
    {
        return Err(invalid("JOIN IN subquery has an unsupported shape"));
    }
    let inner = &subquery.inner;
    let inner_key = match (&inner.projection, subquery.negated) {
        (
            crate::Projection::GroupedAggregate {
                column,
                function: crate::AggregateFunction::Sum,
                ..
            },
            false,
        ) if inner.hidden_aggregate => column,
        (crate::Projection::Columns(columns), true)
            if columns.len() == 1 && !inner.hidden_aggregate =>
        {
            &columns[0]
        }
        (crate::Projection::Columns(columns), false)
            if columns.len() == 1 && !inner.hidden_aggregate =>
        {
            &columns[0]
        }
        _ => return Err(invalid("JOIN IN subquery has an unsupported projection")),
    };
    if !matches!(inner.source, Some(crate::Source::Table(_)))
        || inner
            .predicates
            .iter()
            .chain(inner.or_predicates.iter().flatten())
            .any(|predicate| matches!(predicate, crate::Predicate::ColumnComparison { .. }))
    {
        return Err(invalid(
            "JOIN IN subquery requires an uncorrelated stored table",
        ));
    }
    let inner_metadata = inspect_query_source(inner_path)?;
    let inner_column = inner_metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *inner_key)
        .ok_or_else(|| invalid("JOIN IN subquery key is missing"))?;
    let mut outer_type = None;
    for path in outer_paths {
        let metadata = inspect_query_source(path)?;
        if let Some(column) = metadata
            .columns
            .iter()
            .find(|column| column.name == subquery.column.name)
        {
            if outer_type.replace(column.data_type.clone()).is_some() {
                return Err(invalid("ambiguous JOIN IN column"));
            }
        }
    }
    let outer_type = outer_type.ok_or_else(|| invalid("unknown JOIN IN column"))?;
    let kind = typed::base_type(&inner_column.data_type);
    let integer_key = matches!(
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
    );
    if kind != typed::base_type(&outer_type) || (!integer_key && kind != "String") {
        return Err(invalid(
            "JOIN IN subquery requires matching integer or String key types",
        ));
    }
    let mut inner = (**inner).clone();
    inner.source = Some(crate::Source::Table("part".into()));
    let mut output = SubqueryOutput(Vec::new());
    super::execute(&inner, Some(inner_path), &mut output)?;
    let text =
        std::str::from_utf8(&output.0).map_err(|_| invalid("invalid JOIN IN subquery output"))?;
    let mut values = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut contains_null = false;
    for value in text.lines().skip(1) {
        if value == "\\N" {
            contains_null = true;
            continue;
        }
        let key = if kind == "String" {
            String::from_utf8(
                rusthouse_format::decode_tsv_field(value.as_bytes())
                    .map_err(|_| invalid("invalid JOIN IN subquery text"))?,
            )
            .map_err(|_| invalid("invalid JOIN IN subquery text"))?
        } else {
            if value.is_empty()
                || !value
                    .bytes()
                    .enumerate()
                    .all(|(i, byte)| byte.is_ascii_digit() || (i == 0 && byte == b'-'))
            {
                return Err(invalid("invalid JOIN IN subquery integer"));
            }
            value.to_owned()
        };
        if seen.insert(key.clone()) {
            if values.len() >= MAX_IN_SUBQUERY_KEYS {
                return Err(invalid("JOIN IN subquery supports at most 100000 keys"));
            }
            values.push(key);
        }
    }
    let name = subquery.column.name.clone();
    if subquery.negated {
        // An empty IN predicate is always false, while preserving the other
        // filters so their columns and types are still validated.
        let predicate = if contains_null {
            crate::Predicate::In {
                column: name,
                values: Vec::new(),
            }
        } else {
            crate::Predicate::NotIn {
                column: name,
                values,
            }
        };
        let branches = query.scalar_where.get_or_insert_with(|| vec![vec![]]);
        if let Some(branch) = branches.first_mut() {
            branch.push(predicate);
        }
    } else {
        query.scalar_where = Some(vec![vec![crate::Predicate::In {
            column: name,
            values,
        }]]);
    }
    query.in_subquery = None;
    Ok(())
}

/// Execute a chain of three or more bounded joins. No output is written before
/// all input validation, row-limit checks, and final ordering have succeeded.
pub fn execute_multi_join<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    output: &mut W,
) -> Result<(), ExecutionError> {
    execute_multi_join_inner(query, paths, None, None, None, None, false, output)
}

/// Execute a complete correlated join plan. Callers hold a read lock across all paths.
pub fn execute_correlated_join<W: Write>(
    plan: &crate::JoinCorrelatedExists,
    paths: &[PathBuf],
    inner_paths: &[PathBuf],
    output: &mut W,
) -> Result<(), ExecutionError> {
    execute_multi_join_inner(
        plan.outer(),
        paths,
        Some((&plan.filters, &plan.expression, inner_paths)),
        None,
        None,
        None,
        false,
        output,
    )
}

/// Execute a correlated AVG plan while callers hold the database read lock.
pub fn execute_correlated_avg<W: Write>(
    plan: &crate::JoinCorrelatedAvg,
    paths: &[PathBuf],
    inner: &std::path::Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let inner_paths = [inner.to_path_buf()];
    execute_correlated_avg_with_paths(plan, paths, &inner_paths, output)
}

/// Execute a correlated AVG plan with either a single table or a grouped inner join.
pub fn execute_correlated_avg_with_paths<W: Write>(
    plan: &crate::JoinCorrelatedAvg,
    paths: &[PathBuf],
    inner_paths: &[PathBuf],
    output: &mut W,
) -> Result<(), ExecutionError> {
    execute_multi_join_inner(
        plan.outer(),
        paths,
        None,
        Some((plan, inner_paths)),
        None,
        None,
        false,
        output,
    )
}

/// Execute a correlated MIN plan while callers hold a database read lock.
pub fn execute_correlated_min<W: Write>(
    plan: &crate::JoinCorrelatedMin,
    paths: &[PathBuf],
    inner_paths: &[PathBuf],
    output: &mut W,
) -> Result<(), ExecutionError> {
    execute_multi_join_inner(
        plan.outer(),
        paths,
        None,
        None,
        Some((plan, inner_paths)),
        None,
        false,
        output,
    )
}

/// Execute a correlated SUM plan while callers hold a database read lock.
pub fn execute_correlated_sum<W: Write>(
    plan: &crate::JoinCorrelatedSum,
    paths: &[PathBuf],
    inner: &std::path::Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    execute_multi_join_inner(
        plan.outer(),
        paths,
        None,
        None,
        None,
        Some((plan, inner)),
        false,
        output,
    )
}

fn execute_nested_correlated_sum<W: Write>(
    plan: &crate::JoinCorrelatedSum,
    paths: &[PathBuf],
    inner: &std::path::Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    execute_multi_join_inner(
        plan.outer(),
        paths,
        None,
        None,
        None,
        Some((plan, inner)),
        true,
        output,
    )
}

#[allow(clippy::too_many_arguments)]

#[cfg(test)]
#[path = "multi_join/bucket_tests.rs"]
mod bucket_tests;

#[cfg(test)]
#[path = "multi_join/chained_join_reusable_probe_tests.rs"]
mod chained_join_reusable_probe_tests;

#[cfg(test)]
mod reusable_chain_keys_tests {
    use super::*;

    #[test]
    fn reused_text_keys_agree_with_fresh_materialized_keys() {
        let rows = (0..200usize)
            .map(|index| {
                if index % 17 == 0 {
                    vec![typed::Cell::Null]
                } else {
                    vec![typed::Cell::Text(format!("key-{}", index % 5).into_bytes())]
                }
            })
            .collect::<Vec<_>>();
        let mut fresh = HashMap::<Vec<typed::Key>, Bucket>::new();
        let mut reused = HashMap::<Vec<typed::Key>, Bucket>::new();
        let mut scratch = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            if let Some(key) = row_key(row, &[0]) {
                match fresh.get_mut(key.as_slice()) {
                    Some(bucket) => bucket.push(index),
                    None => {
                        fresh.insert(key, Bucket::One(index));
                    }
                }
            }
            if row_key_into(row, &[0], &mut scratch) {
                match reused.get_mut(scratch.as_slice()) {
                    Some(bucket) => bucket.push(index),
                    None => {
                        reused.insert(scratch.clone(), Bucket::One(index));
                    }
                }
            }
        }
        assert_eq!(fresh.len(), 5);
        assert_eq!(reused.len(), 5);
        for (key, expected) in &fresh {
            let actual = reused.get(key.as_slice()).unwrap();
            assert_eq!(
                actual.iter().copied().collect::<Vec<_>>(),
                expected.iter().copied().collect::<Vec<_>>()
            );
        }
        let mut count = 0;
        for row in &rows {
            if row_key_into(row, &[0], &mut scratch) {
                count += reused.get(scratch.as_slice()).unwrap().iter().len();
            }
        }
        assert!(count > rows.len(), "duplicates increase ALL JOIN count");
    }
}

#[cfg(test)]
mod grouped_in_binding_tests {
    use super::*;

    #[test]
    fn rejects_unbound_and_mutated_join_filters_before_reading_files() {
        let sql = "SELECT ak, SUM(value) AS total FROM a JOIN b ON a.ak = b.bk JOIN c ON b.bk = c.ck WHERE ak IN (SELECT ik FROM keys GROUP BY ik HAVING SUM(quantity) > 3) GROUP BY ak";
        let query = crate::parse_join(sql).unwrap().unwrap();
        let paths = vec![
            PathBuf::from("missing-a"),
            PathBuf::from("missing-b"),
            PathBuf::from("missing-c"),
        ];
        let error = execute_multi_join(&query, &paths, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("must be bound"), "{error}");
        let mut altered = query.clone();
        altered.where_in_filters.push((
            JoinColumn {
                relation: String::new(),
                name: "ak".into(),
            },
            vec!["1".into()],
        ));
        let error =
            bind_join_in_subquery(&mut altered, std::path::Path::new("missing-inner"), &paths)
                .unwrap_err();
        assert!(error.to_string().contains("unsupported shape"), "{error}");
        let mut altered = query;
        altered.where_like_filters.push((
            JoinColumn {
                relation: String::new(),
                name: "ak".into(),
            },
            "%".into(),
            false,
            false,
        ));
        let error =
            bind_join_in_subquery(&mut altered, std::path::Path::new("missing-inner"), &paths)
                .unwrap_err();
        assert!(error.to_string().contains("unsupported shape"), "{error}");
    }
}

#[cfg(test)]
mod grouped_distinct_plan_tests {
    use super::*;

    #[test]
    fn rejects_ignored_filters_before_reading_sources() {
        let sql = "SELECT ak, COUNT(DISTINCT value) AS n FROM a JOIN b ON a.ak = b.bk GROUP BY ak";
        let query = crate::parse_join(sql).unwrap().unwrap();
        let paths = vec![PathBuf::from("missing-a"), PathBuf::from("missing-b")];
        let mut altered = query.clone();
        altered.where_in_filters.push((
            JoinColumn {
                relation: String::new(),
                name: "ak".into(),
            },
            vec!["1".into()],
        ));
        let error = execute_multi_join(&altered, &paths, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("INNER grouped plan"), "{error}");
        let mut altered = query;
        altered.where_like_filters.push((
            JoinColumn {
                relation: String::new(),
                name: "ak".into(),
            },
            "%".into(),
            false,
            false,
        ));
        let error = execute_multi_join(&altered, &paths, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("INNER grouped plan"), "{error}");
    }
}

#[cfg(test)]
mod not_in_binding_tests {
    use super::*;

    #[test]
    fn rejects_multiple_residual_branches_before_reading_sources() {
        let sql = "SELECT ak, COUNT(DISTINCT value) AS n FROM a JOIN b ON a.ak = b.bk WHERE value NOT IN (SELECT ik FROM keys) GROUP BY ak";
        let mut query = crate::parse_join(sql).unwrap().unwrap();
        query.scalar_where = Some(vec![vec![], vec![]]);
        let paths = vec![PathBuf::from("missing-a"), PathBuf::from("missing-b")];
        let error =
            bind_join_in_subquery(&mut query, std::path::Path::new("missing-inner"), &paths)
                .unwrap_err();
        assert!(error.to_string().contains("unsupported shape"), "{error}");
    }
}
