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
mod reorder;
mod row_filters;
mod streaming;
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
fn execute_multi_join_inner<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    exists_plan: Option<(&[crate::JoinExistsFilter], &[Vec<usize>], &[PathBuf])>,
    avg_plan: Option<(&crate::JoinCorrelatedAvg, &[PathBuf])>,
    min_plan: Option<(&crate::JoinCorrelatedMin, &[PathBuf])>,
    sum_plan: Option<(&crate::JoinCorrelatedSum, &std::path::Path)>,
    stream_buffered_sum: bool,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if query
        .grouped_formula
        .as_ref()
        .is_some_and(|formula| formula.sums.is_empty() || formula.sums.len() > 8)
    {
        return Err(invalid("grouped formula requires between 1 and 8 sums"));
    }
    if query.grouped_formula.is_some()
        && (query.grouped_count.is_none()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
            || query.grouped_sum.is_some()
            || query.grouped_min.is_some()
            || query.grouped_distinct.is_some()
            || query.grouped_count_column.is_some()
            || query.aggregate.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
            || !query.grouped_case_counts.is_empty()
            || query.grouped_sum_having.is_some())
    {
        return Err(invalid("grouped formula requires an INNER grouped plan"));
    }
    if query.grouped_min.is_some()
        && (query.grouped_count.is_none()
            || query.grouped_sum.is_some()
            || query.grouped_distinct.is_some()
            || query.grouped_count_column.is_some()
            || query.aggregate.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
            || !query.grouped_case_counts.is_empty()
            || query.grouped_sum_having.is_some()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner))
    {
        return Err(invalid("grouped MIN requires an INNER grouped plan"));
    }
    if query.on_right_like.is_some() {
        return Err(invalid(
            "JOIN ON LIKE requires the two-table LEFT JOIN executor",
        ));
    }
    if query.grouped_count_column.is_some() {
        return Err(invalid(
            "grouped COUNT(column) requires the two-table JOIN executor",
        ));
    }
    if query
        .grouped_sum_having
        .as_ref()
        .is_some_and(|having| having.subquery.is_some())
    {
        return Err(invalid(
            "JOIN HAVING subquery must be bound before execution",
        ));
    }
    if query.in_subquery.is_some() {
        return Err(invalid("JOIN IN subquery must be bound before execution"));
    }
    if query.grouped_distinct.is_some()
        && (query.grouped_count.is_none()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
            || query.grouped_sum.is_some()
            || query.aggregate.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
            || !query.grouped_case_counts.is_empty()
            || !query.where_in_filters.is_empty()
            || !query.where_like_filters.is_empty())
    {
        return Err(invalid(
            "grouped JOIN COUNT DISTINCT requires an INNER grouped plan",
        ));
    }
    if paths.len() != query.steps.len() + 2 {
        return Err(invalid("JOIN source count differs from its plan"));
    }
    let mut relations = Vec::with_capacity(paths.len());
    let mut width = 0usize;
    for (index, path) in paths.iter().enumerate() {
        let metadata = inspect_query_source(path)?;
        let mut names = metadata
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let mut types = metadata
            .columns
            .iter()
            .map(|column| column.data_type.clone())
            .collect::<Vec<_>>();
        let alias = match index {
            0 => query.left_alias.clone(),
            1 => query.right_alias.clone(),
            _ => query.steps[index - 2].alias.clone(),
        };
        let physical_columns = names.len();
        let mut virtual_columns = Vec::new();
        for virtual_column in query
            .virtual_columns
            .iter()
            .filter(|column| column.relation == alias)
        {
            if names.iter().any(|name| name == &virtual_column.name) {
                return Err(invalid(
                    "derived JOIN column conflicts with a stored column",
                ));
            }
            let source_index = names
                .iter()
                .position(|name| name == &virtual_column.source)
                .ok_or_else(|| invalid("derived JOIN source column does not exist"))?;
            let virtual_index = names.len();
            names.push(virtual_column.name.clone());
            types.push("String".into());
            virtual_columns.push(BoundVirtualColumn {
                index: virtual_index,
                source_index,
                start: virtual_column.start,
                length: virtual_column.length,
            });
        }
        let start = width;
        width = width
            .checked_add(names.len())
            .ok_or_else(|| invalid("JOIN column count overflow"))?;
        if width > MAX_CHAIN_COLUMNS {
            return Err(invalid("JOIN supports at most 256 source columns"));
        }
        relations.push(Relation {
            alias,
            names,
            types,
            physical_columns,
            virtual_columns,
            start,
            rows: metadata.rows,
        });
    }
    // EXISTS columns are rebound by relation alias after reordering.
    if avg_plan.is_none() && min_plan.is_none() && sum_plan.is_none() {
        let sizes = relations
            .iter()
            .map(|relation| relation.rows)
            .collect::<Vec<_>>();
        if let Some((reordered, order)) = reorder::plan(query, &sizes) {
            let reordered_paths = order
                .into_iter()
                .map(|index| paths[index].clone())
                .collect::<Vec<_>>();
            return execute_multi_join_inner(
                &reordered,
                &reordered_paths,
                exists_plan,
                None,
                None,
                None,
                stream_buffered_sum,
                output,
            );
        }
    }
    let exists = if let Some((filters, _, inner_paths)) = exists_plan {
        if (query.grouped_count.is_none() && query.scalar_aggregates.is_empty())
            || query.aggregate.is_some()
            || query.grouped_sum.is_some()
            || query.grouped_distinct.is_some()
            || query.grouped_count_column.is_some()
            || !query.grouped_case_counts.is_empty()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        {
            return Err(invalid(
                "correlated JOIN EXISTS requires an INNER aggregate",
            ));
        }
        exists::bind(filters, inner_paths, &relations)?
    } else {
        Vec::new()
    };
    let exists_expression = exists_plan.map_or(&[][..], |(_, expression, _)| expression);
    let avg = if let Some((plan, inner_paths)) = avg_plan {
        if (query.scalar_formula.is_none() && query.scalar_sum.is_none() && query.columns.is_none())
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        {
            return Err(invalid(
                "correlated AVG requires a scalar INNER JOIN aggregate",
            ));
        }
        Some(correlated_avg::bind(plan, inner_paths, &relations)?)
    } else {
        None
    };

    if !query.grouped_key_aliases.is_empty() {
        let simple_projection = query.grouped_count.is_none()
            && !query.grouped_keys_only
            && query.aggregate.is_none()
            && query.scalar_aggregates.is_empty()
            && query.scalar_sum.is_none()
            && query.scalar_formula.is_none()
            && query.grouped_sum.is_none()
            && query.grouped_min.is_none()
            && query.grouped_formula.is_none()
            && query.grouped_window_ratio.is_none();
        let simple_projection_outer_chain = simple_projection
            && matches!(query.kind, JoinType::Inner | JoinType::Left)
            && query
                .steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left));
        let grouped_sum_outer_chain = query.grouped_sum.is_some()
            && query.grouped_count.is_some()
            && query
                .columns
                .as_ref()
                .is_some_and(|columns| !columns.is_empty())
            && matches!(query.kind, JoinType::Inner | JoinType::Left)
            && query
                .steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left));
        if !(query.grouped_count.is_some() || simple_projection)
            || (!(simple_projection_outer_chain || grouped_sum_outer_chain)
                && (query.kind != JoinType::Inner
                    || query.steps.iter().any(|step| step.kind != JoinType::Inner)))
        {
            return Err(invalid(
                "JOIN key aliases require a simple or grouped INNER join",
            ));
        }
        for (position, alias) in &query.grouped_key_aliases {
            if query
                .columns
                .as_ref()
                .is_none_or(|columns| *position >= columns.len())
                || (query.grouped_count.is_some()
                    && relations.iter().any(|relation| {
                        relation
                            .names
                            .iter()
                            .any(|name| name.eq_ignore_ascii_case(alias))
                    }))
            {
                return Err(invalid("JOIN key alias conflicts with a source column"));
            }
        }
    }

    if let Some((position, alias)) = &query.grouped_year {
        if relations.iter().any(|relation| {
            relation
                .names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(alias))
        }) {
            return Err(invalid("EXTRACT YEAR alias conflicts with a source column"));
        }
        let column = query
            .columns
            .as_ref()
            .and_then(|columns| columns.get(*position))
            .ok_or_else(|| invalid("EXTRACT YEAR requires a projected column"))?;
        if query.grouped_count.is_none()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        {
            return Err(invalid("EXTRACT YEAR requires a grouped INNER join"));
        }
        let kind = typed::base_type(column_type(&relations, column_index(&relations, column)?));
        if !matches!(kind, "Date" | "Date32") {
            return Err(invalid("EXTRACT YEAR requires Date or Date32"));
        }
    }

    let min_filter = if let Some((plan, inner_paths)) = min_plan {
        if query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
            || query.aggregate.is_some()
            || query.grouped_count.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
        {
            return Err(invalid("correlated MIN requires an INNER JOIN projection"));
        }
        Some(correlated_min::bind(
            plan,
            inner_paths,
            &relations,
            query.limit == Some(0),
        )?)
    } else {
        None
    };

    let sum_filter = if let Some((plan, inner)) = sum_plan {
        if query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
            || query.aggregate.is_some()
            || query.grouped_count.is_some()
            || query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
        {
            return Err(invalid("correlated SUM requires an INNER JOIN projection"));
        }
        Some(correlated_sum::bind(
            plan,
            inner,
            &relations,
            query.limit == Some(0),
        )?)
    } else {
        None
    };

    let initial_step = JoinStep {
        table: query.right.clone(),
        alias: query.right_alias.clone(),
        kind: query.kind,
        any: query.any,
        keys: query.keys.clone(),
        using_keys: query.using_keys.clone(),
    };
    let scalar_metric = match (&query.scalar_sum, &query.scalar_sum_qualifier) {
        (Some(crate::ScalarExpr::Column(name)), Some(relation)) => {
            Some(crate::join::GroupedSum::Column(JoinColumn {
                relation: relation.clone(),
                name: name.clone(),
            }))
        }
        (Some(expression), None) => Some(crate::join::GroupedSum::Expression(expression.clone())),
        (None, None) => None,
        _ => return Err(invalid("invalid qualified scalar SUM")),
    };
    let bind_sum = |metric: &crate::join::GroupedSum| -> Result<BoundSum, ExecutionError> {
        let (expression, names) = match metric {
            crate::join::GroupedSum::Column(column) => {
                let index = column_index(&relations, column)?;
                (
                    super::multi_aggregate::BoundExpr::Column(index),
                    vec![index],
                )
            }
            crate::join::GroupedSum::Expression(expression) => {
                let mut columns = Vec::new();
                expression.referenced_columns(&mut columns);
                let mut indices = HashMap::new();
                let mut names = Vec::new();
                for name in columns {
                    let (relation, column_name) = name.split_once('\0').unwrap_or(("", &name));
                    let column = JoinColumn {
                        relation: relation.to_owned(),
                        name: column_name.to_owned(),
                    };
                    let index = column_index(&relations, &column)?;
                    indices.insert(name, index);
                    names.push(index);
                }
                (
                    super::multi_aggregate::BoundExpr::bind(expression, &indices)?,
                    names,
                )
            }
        };
        validate_sum_expression(&expression, &relations)?;
        Ok(BoundSum {
            expression,
            indices: names,
        })
    };
    let min_metric = query
        .grouped_min
        .as_ref()
        .map(|column| crate::join::GroupedSum::Column(column.clone()));
    let grouped_sum = query
        .grouped_sum
        .as_ref()
        .or(scalar_metric.as_ref())
        .or(min_metric.as_ref())
        .map(bind_sum)
        .transpose()?;
    let grouped_sum_extras = query
        .grouped_sum_extras
        .iter()
        .map(|(metric, _)| bind_sum(metric))
        .collect::<Result<Vec<_>, _>>()?;
    let formula = query
        .scalar_formula
        .as_ref()
        .or(query.grouped_formula.as_ref())
        .map(|formula| -> Result<BoundFormula, ExecutionError> {
            let sums = formula
                .sums
                .iter()
                .map(|expression| {
                    bind_sum(&crate::join::GroupedSum::Expression(expression.clone()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let indices = (0..sums.len())
                .map(|index| (format!("__join_sum_{index}"), index))
                .collect::<HashMap<_, _>>();
            let expression =
                super::multi_aggregate::BoundExpr::bind(&formula.expression, &indices)?;
            Ok(BoundFormula { sums, expression })
        })
        .transpose()?;
    let distinct_index = query
        .grouped_distinct
        .as_ref()
        .map(|column| {
            let index = column_index(&relations, column)?;
            let kind = typed::base_type(column_type(&relations, index));
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
            ) {
                return Err(invalid(
                    "grouped JOIN COUNT DISTINCT requires an integer column",
                ));
            }
            Ok(index)
        })
        .transpose()?;
    let count_column = if let Some(crate::JoinAggregate::CountColumn(column)) = &query.aggregate {
        let index = column_index(&relations, column)?;
        let header = query
            .aggregate_alias
            .clone()
            .unwrap_or_else(|| format!("count({})", column.display_name()));
        Some((index, header))
    } else {
        None
    };
    // Bind predicates before early returns, including LIMIT 0.
    let comparisons = query
        .where_comparison
        .iter()
        .chain(&query.where_comparisons_extra)
        .map(|(column, operator, literal)| {
            let index = column_index(&relations, column)?;
            let value = typed::parse_literal(column_type(&relations, index), literal)?;
            Ok((index, *operator, value))
        })
        .collect::<Result<Vec<_>, ExecutionError>>()?;

    let column_comparisons = query
        .where_column_comparisons
        .iter()
        .map(|(left, operator, right)| {
            let left = column_index(&relations, left)?;
            let right = column_index(&relations, right)?;
            let left_type = typed::base_type(column_type(&relations, left));
            let right_type = typed::base_type(column_type(&relations, right));
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
            Ok((left, *operator, right))
        })
        .collect::<Result<Vec<_>, ExecutionError>>()?;

    // Bind both source columns and require textual types before early returns.
    let substring_comparisons = query
        .where_substring_comparisons
        .iter()
        .map(
            |(left, start, length, operator, right, right_start, right_length)| {
                let left_index = column_index(&relations, left)?;
                let right_index = column_index(&relations, right)?;
                if typed::base_type(column_type(&relations, left_index)) != "String"
                    || typed::base_type(column_type(&relations, right_index)) != "String"
                {
                    return Err(invalid("SUBSTRING JOIN comparisons require String columns"));
                }
                Ok((
                    left_index,
                    *start,
                    *length,
                    *operator,
                    right_index,
                    *right_start,
                    *right_length,
                ))
            },
        )
        .collect::<Result<Vec<BoundSubstringComparison>, ExecutionError>>()?;

    // Two-table parsers may store IN and LIKE separately from boolean branches.
    // Correlated plans also use this executor, so retain those conjuncts here.
    let row_filters = if query.scalar_where.is_some()
        || !query.where_in_filters.is_empty()
        || !query.where_like_filters.is_empty()
    {
        let mut branches = query.scalar_where.clone().unwrap_or_else(|| vec![vec![]]);
        let column_name = |column: &JoinColumn| {
            if column.relation.is_empty() {
                column.name.clone()
            } else {
                format!("{}\0{}", column.relation, column.name)
            }
        };
        for branch in &mut branches {
            branch.extend(query.where_in_filters.iter().map(|(column, values)| {
                crate::Predicate::In {
                    column: column_name(column),
                    values: values.clone(),
                }
            }));
            branch.extend(query.where_like_filters.iter().map(
                |(column, pattern, case_insensitive, negated)| crate::Predicate::Like {
                    column: column_name(column),
                    pattern: pattern.clone(),
                    case_insensitive: *case_insensitive,
                    negated: *negated,
                },
            ));
        }
        Some(row_filters::RowFilters::bind(&branches, &relations)?)
    } else {
        None
    };

    // Validate the complete output schema without materializing rows.
    if query.limit == Some(0)
        || ((query.aggregate.is_some()
            || query.scalar_sum.is_some()
            || !query.scalar_aggregates.is_empty()
            || query.scalar_formula.is_some())
            && query.offset > 0)
    {
        for (index, join) in std::iter::once(&initial_step)
            .chain(query.steps.iter())
            .enumerate()
        {
            let width = relations[index + 1].start;
            step_keys(join, &relations[..=index + 1], width)?;
        }
        if let Some((column, _)) = &query.where_null {
            column_index(&relations, column)?;
        }
        for (column, _, _) in &query.order_by {
            if !(column.is_count_order()
                && (query.grouped_count.is_some()
                    || query.aggregate == Some(crate::JoinAggregate::CountStar)
                    || !query.scalar_aggregates.is_empty()
                    || query.scalar_sum.is_some()))
                && column.scalar_order_index().is_none()
            {
                column_index(&relations, column)?;
            }
        }
        if !query.scalar_aggregates.is_empty() {
            write_scalar_aggregate_header(query, output)?;
        } else if query.scalar_sum.is_some() || query.scalar_formula.is_some() {
            write_scalar_sum(query, None, false, output)?;
        } else if let Some((_, header)) = &count_column {
            writeln!(output, "{header}")?;
        } else if query.aggregate == Some(crate::JoinAggregate::CountStar) {
            writeln!(
                output,
                "{}",
                query.aggregate_alias.as_deref().unwrap_or("count_star()")
            )?;
        } else {
            let selected = output_columns(query, &relations, &initial_step)?;
            if query.grouped_count.is_some() {
                write_group_header(query, &selected, output)?;
            } else {
                write_join_header(output, &selected)?;
            }
        }
        return Ok(());
    }

    // A left-deep INNER chain can stream its large fact side when each
    // right-hand build table fits the shared chain memory budget. Grouped
    // CASE metrics also support LEFT stages for NULL checks.
    let streamable_join_chain = (query.kind == JoinType::Inner
        && query.steps.iter().all(|step| step.kind == JoinType::Inner))
        || (!query.grouped_case_counts.is_empty()
            && matches!(query.kind, JoinType::Inner | JoinType::Left)
            && query
                .steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left)))
        || (query.grouped_sum.is_some()
            && query.grouped_count.is_some()
            && matches!(query.kind, JoinType::Inner | JoinType::Left)
            && query
                .steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left)));
    if streamable_join_chain
        && (query.order_by.is_empty()
            || query.grouped_count.is_some()
            || (!query.scalar_projections.is_empty() && query.grouped_count.is_none())
            || (!query.scalar_aggregates.is_empty()
                && query
                    .order_by
                    .iter()
                    .all(|(column, _, _)| column.is_count_order()))
            || (query.scalar_sum.is_some()
                && query
                    .order_by
                    .iter()
                    .all(|(column, _, _)| column.is_count_order())))
        && query.aggregate.is_none()
        && min_filter.is_none()
        && (sum_filter.is_none() || stream_buffered_sum)
    {
        return streaming::execute(
            query,
            paths,
            &relations,
            &initial_step,
            &comparisons,
            &column_comparisons,
            &substring_comparisons,
            grouped_sum.as_ref(),
            &grouped_sum_extras,
            formula.as_ref(),
            distinct_index,
            row_filters.as_ref(),
            &exists,
            exists_expression,
            avg.as_ref(),
            sum_filter.as_ref(),
            output,
        );
    }

    // Resolve the WHERE column across all relations before materialization.
    // Filtering early is safe only at the final JOIN, after null extension.
    let output_filter = query
        .where_null
        .as_ref()
        .map(|(column, not_null)| column_index(&relations, column).map(|index| (index, *not_null)))
        .transpose()?;
    let required = required_physical_columns(
        query,
        &relations,
        &initial_step,
        row_filters.as_ref(),
        avg.as_ref(),
        min_filter.as_ref(),
        sum_filter.as_ref(),
    )?;
    let min_filter_relation = min_filter
        .as_ref()
        .map(|filter| {
            relations
                .iter()
                .position(|relation| {
                    filter.required_row_width() <= relation.start + relation.names.len()
                })
                .ok_or_else(|| invalid("correlated MIN columns exceed the JOIN row width"))
        })
        .transpose()?;
    // Correlated MIN needs a materialized ordered result. Its projected inputs
    // can use the same bounded build-row cap as streaming joins; retained
    // bytes and intermediate matches are still checked independently.
    let input_row_limit = if min_filter.is_some()
        || (query.aggregate == Some(crate::JoinAggregate::CountStar)
            && query.kind == JoinType::Inner
            && query.steps.iter().all(|step| step.kind == JoinType::Inner))
    {
        crate::join::MAX_STREAM_JOIN_BUILD_ROWS
    } else {
        MAX_INPUT_ROWS
    };
    let inner_join_plan = query.kind == JoinType::Inner
        && query.steps.iter().all(|step| step.kind == JoinType::Inner);
    let first_relation_filters = if inner_join_plan {
        relation_comparison_filters(&comparisons, 0, relations[0].physical_columns)
    } else {
        Vec::new()
    };
    let first_row_filter = row_filters
        .as_ref()
        .filter(|filters| {
            inner_join_plan && filters.can_prefilter_relation(0, relations[0].names.len())
        })
        .map(|filters| {
            let conditions = filters.prefilter_conditions(0, relations[0].names.len());
            (filters, conditions)
        });
    let mut rows = if first_relation_filters.is_empty() && first_row_filter.is_none() {
        read_projected_build_rows(
            &paths[0],
            &relations[0].names,
            &required[0],
            relations[0].rows,
            input_row_limit,
            MAX_STREAM_BUILD_BYTES,
        )?
    } else {
        let mut predicate = |row: &[typed::Cell]| {
            passes_relation_comparison_filters(row, &first_relation_filters)
                && first_row_filter
                    .as_ref()
                    .map_or(true, |(filters, conditions)| {
                        filters.passes_relation(row, 0, conditions)
                    })
        };
        read_projected_build_rows_matching(
            &paths[0],
            &relations[0].names,
            &required[0],
            relations[0].rows,
            input_row_limit,
            MAX_STREAM_BUILD_BYTES,
            &mut predicate,
        )?
    };
    if min_filter_relation == Some(0) {
        let filter = min_filter
            .as_ref()
            .ok_or_else(|| invalid("correlated MIN relation has no filter"))?;
        let mut filtered = Vec::with_capacity(rows.len());
        for row in rows {
            if filter.passes(&row)? {
                filtered.push(row);
            }
        }
        rows = filtered;
    }
    for (step_index, step) in std::iter::once(&initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let right_relation = &relations[step_index + 1];
        let left_width = right_relation.start;
        let (left_keys, right_keys) = step_keys(step, &relations[..=step_index + 1], left_width)?;
        let apply_min_filter = min_filter_relation == Some(step_index + 1);
        // INNER and LEFT cannot create rows from an empty intermediate.
        // Keep the source row limit even when its payload is not needed.
        if rows.is_empty() && matches!(step.kind, JoinType::Inner | JoinType::Left) {
            if right_relation.rows > input_row_limit {
                return Err(invalid(format!(
                    "JOIN supports at most {input_row_limit} input rows per table"
                )));
            }
            continue;
        }
        let right_filters = if inner_join_plan {
            relation_comparison_filters(
                &comparisons,
                right_relation.start,
                right_relation.physical_columns,
            )
        } else {
            Vec::new()
        };
        let right_row_filter = row_filters
            .as_ref()
            .filter(|filters| {
                inner_join_plan
                    && filters
                        .can_prefilter_relation(right_relation.start, right_relation.names.len())
            })
            .map(|filters| {
                let conditions =
                    filters.prefilter_conditions(right_relation.start, right_relation.names.len());
                (filters, conditions)
            });
        let right_rows = if right_filters.is_empty() && right_row_filter.is_none() {
            read_projected_build_rows(
                &paths[step_index + 1],
                &right_relation.names,
                &required[step_index + 1],
                right_relation.rows,
                input_row_limit,
                MAX_STREAM_BUILD_BYTES,
            )?
        } else {
            let mut predicate = |row: &[typed::Cell]| {
                passes_relation_comparison_filters(row, &right_filters)
                    && right_row_filter
                        .as_ref()
                        .map_or(true, |(filters, conditions)| {
                            filters.passes_relation(row, right_relation.start, conditions)
                        })
            };
            read_projected_build_rows_matching(
                &paths[step_index + 1],
                &right_relation.names,
                &required[step_index + 1],
                right_relation.rows,
                input_row_limit,
                MAX_STREAM_BUILD_BYTES,
                &mut predicate,
            )?
        };
        let mut hash: HashMap<Vec<typed::Key>, Bucket> = HashMap::new();
        let mut hash_bytes = 0usize;
        let mut build_key = Vec::with_capacity(right_keys.len());
        for (index, row) in right_rows.iter().enumerate() {
            if !row_key_into(row, &right_keys, &mut build_key) {
                continue;
            }
            // Lookup does not own the key. Skewed dimensions allocate only
            // for distinct hash entries instead of every duplicate row.
            if let Some(bucket) = hash.get_mut(build_key.as_slice()) {
                charge_hash_bytes(&mut hash_bytes, std::mem::size_of::<usize>() * 2)?;
                bucket.push(index);
            } else {
                charge_hash_bytes(&mut hash_bytes, hash_key_bytes(&build_key)?)?;
                hash.insert(build_key.clone(), Bucket::One(index));
            }
        }
        // COUNT(*) over a validated all-INNER chain needs only the final
        // hash-bucket multiplicities. No final joined rows are retained.
        if query.aggregate == Some(crate::JoinAggregate::CountStar)
            && query.kind == JoinType::Inner
            && query.steps.iter().all(|step| step.kind == JoinType::Inner)
            && avg.is_none()
            && step_index == query.steps.len()
            && comparisons.is_empty()
            && column_comparisons.is_empty()
            && row_filters.is_none()
        {
            let mut count = 0u64;
            let mut count_key = Vec::with_capacity(left_keys.len());
            for row in &rows {
                if row_key_into(row, &left_keys, &mut count_key) {
                    if let Some(matches) = hash.get(count_key.as_slice()) {
                        // The final COUNT fast path must apply the same
                        // IS NULL / IS NOT NULL filter as materialized joins.
                        // With ANY, inspect only the first matched dimension
                        // row: filtering all matches changes first-match rules.
                        let multiplicity = match output_filter {
                            Some((filter_index, not_null)) if filter_index < left_width => {
                                if matches!(row[filter_index], typed::Cell::Null) == not_null {
                                    0
                                } else if step.any {
                                    1
                                } else {
                                    matches.iter().len()
                                }
                            }
                            Some((filter_index, not_null)) => matches
                                .iter()
                                .take(if step.any { 1 } else { usize::MAX })
                                .filter(|&&right_index| {
                                    matches!(
                                        right_rows[right_index][filter_index - left_width],
                                        typed::Cell::Null
                                    ) != not_null
                                })
                                .count(),
                            None => {
                                if step.any {
                                    1
                                } else {
                                    matches.iter().len()
                                }
                            }
                        };
                        count = count
                            .checked_add(
                                u64::try_from(multiplicity)
                                    .map_err(|_| invalid("JOIN count overflow"))?,
                            )
                            .ok_or_else(|| invalid("JOIN count overflow"))?;
                    }
                }
            }
            writeln!(
                output,
                "{}\n{count}",
                query.aggregate_alias.as_deref().unwrap_or("count_star()")
            )?;
            return Ok(());
        }
        // Most fact-to-dimension joins preserve the incoming row count.
        let mut next = Vec::with_capacity(rows.len().min(MAX_RESULT_ROWS));
        let mut retained = 0usize;
        let mut matched_right = matches!(step.kind, JoinType::Right | JoinType::Full)
            .then(|| vec![false; right_rows.len()]);
        let filter = if step_index == query.steps.len() {
            output_filter
        } else {
            None
        };
        let mut probe_key = Vec::with_capacity(left_keys.len());
        for row in &rows {
            let mut matched = false;
            if row_key_into(row, &left_keys, &mut probe_key) {
                if let Some(matches) = hash.get(probe_key.as_slice()) {
                    for &match_index in
                        matches
                            .iter()
                            .take(if step.any && step.kind != JoinType::Right {
                                1
                            } else {
                                usize::MAX
                            })
                    {
                        if step.any
                            && step.kind == JoinType::Right
                            && matched_right
                                .as_ref()
                                .is_some_and(|flags| flags[match_index])
                        {
                            continue;
                        }
                        matched = true;
                        if let Some(flags) = matched_right.as_mut() {
                            flags[match_index] = true;
                        }
                        let mut joined =
                            Vec::with_capacity(left_width + right_relation.names.len());
                        joined.extend_from_slice(row);
                        joined.extend_from_slice(&right_rows[match_index]);
                        if apply_min_filter {
                            let filter = min_filter
                                .as_ref()
                                .ok_or_else(|| invalid("correlated MIN relation has no filter"))?;
                            if !filter.passes(&joined)? {
                                continue;
                            }
                        }
                        if step_index != query.steps.len()
                            || (comparisons.iter().all(|(index, operator, literal)| {
                                typed::matches_comparison(&joined[*index], literal, *operator)
                            }) && column_comparisons.iter().all(|(left, operator, right)| {
                                typed::matches_comparison(
                                    &joined[*left],
                                    &joined[*right],
                                    *operator,
                                )
                            }) && match &row_filters {
                                Some(filters) => filters.passes(&joined)?,
                                None => true,
                            } && match &sum_filter {
                                Some(filter) => filter.passes(&joined)?,
                                None => true,
                            }) && match &avg {
                                Some(filter) => filter.passes(&joined)?,
                                None => true,
                            }
                        {
                            push_visible_row(&mut next, joined, &mut retained, filter)?;
                        }
                    }
                }
            }
            if !matched && matches!(step.kind, JoinType::Left | JoinType::Full) {
                let mut joined = Vec::with_capacity(left_width + right_relation.names.len());
                joined.extend_from_slice(row);
                joined.resize(left_width + right_relation.names.len(), typed::Cell::Null);
                let passes_filter = step_index != query.steps.len()
                    || match &row_filters {
                        Some(filters) => filters.passes(&joined)?,
                        None => true,
                    };
                if passes_filter {
                    push_visible_row(&mut next, joined, &mut retained, filter)?;
                }
            }
        }
        if let Some(matched_right) = matched_right {
            for (right_index, matched) in matched_right.into_iter().enumerate() {
                if !matched {
                    let mut joined = vec![typed::Cell::Null; left_width];
                    joined.extend_from_slice(&right_rows[right_index]);
                    let passes_filter = step_index != query.steps.len()
                        || match &row_filters {
                            Some(filters) => filters.passes(&joined)?,
                            None => true,
                        };
                    if passes_filter {
                        push_visible_row(&mut next, joined, &mut retained, filter)?;
                    }
                }
            }
        }
        rows = next;
    }

    if !query.order_by.is_empty() && query.aggregate != Some(crate::JoinAggregate::CountStar) {
        let scalar_projections = bind_scalar_projections(query, &relations)?;
        let sort_base = relations
            .iter()
            .map(|relation| relation.names.len())
            .sum::<usize>();
        let ordering_columns = query
            .order_by
            .iter()
            .map(|(column, descending, nulls_first)| {
                let index = column
                    .scalar_order_index()
                    .map(|index| sort_base + index)
                    .map_or_else(|| column_index(&relations, column), Ok)?;
                Ok((index, *descending, *nulls_first))
            })
            .collect::<Result<Vec<_>, ExecutionError>>()?;
        if query
            .order_by
            .iter()
            .any(|(column, _, _)| column.scalar_order_index().is_some())
        {
            for row in &mut rows {
                let values = scalar_projections
                    .iter()
                    .map(|expression| super::multi_aggregate::eval_row_value(expression, row))
                    .collect::<Result<Vec<_>, _>>()?;
                row.extend(values);
            }
        }
        rows.sort_by(|left, right| {
            for (index, descending, nulls_first) in &ordering_columns {
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
    }
    if write_global_case_aggregates(query, &rows, &relations, output)? {
        return Ok(());
    }
    if query.aggregate == Some(crate::JoinAggregate::CountStar) {
        writeln!(
            output,
            "{}",
            query.aggregate_alias.as_deref().unwrap_or("count_star()")
        )?;
        if query.offset == 0 {
            writeln!(output, "{}", rows.len())?;
        }
        return Ok(());
    }
    if let Some((index, header)) = count_column {
        let count = rows
            .iter()
            .filter(|row| !matches!(row[index], typed::Cell::Null))
            .count();
        writeln!(output, "{header}\n{count}")?;
        return Ok(());
    }
    let selected = output_columns(query, &relations, &initial_step)?;
    write_join_header(output, &selected)?;
    let scalar_projections = bind_scalar_projections(query, &relations)?;
    for row in rows
        .iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (position, (index, _, fallback)) in selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            if let Some(expression) = scalar_output_index(*index, scalar_projections.len()) {
                let value =
                    super::multi_aggregate::eval_row_value(&scalar_projections[expression], row)?;
                typed::write_value(output, &value)?;
                continue;
            }
            let source = if matches!(row[*index], typed::Cell::Null) {
                fallback.unwrap_or(*index)
            } else {
                *index
            };
            typed::write_typed_value(output, &row[source], column_type(&relations, source))?;
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

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
