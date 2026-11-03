//! Bounded left-deep execution for chained joins over persistent tables.

mod correlated_avg;
mod exists;
mod row_filters;
mod streaming;
use super::{ExecutionError, typed};
use crate::{JoinColumn, JoinSelect, JoinStep, JoinType};
use rusthouse_storage::inspect_query_source;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

#[cfg(test)]
use super::join::multi_join_row_key as row_key;

use super::join::{
    MAX_INPUT_ROWS, MAX_RESULT_ROWS, MAX_STREAM_BUILD_BYTES,
    multi_join_row_key_into as row_key_into, read_projected_build_rows,
};

const MAX_CHAIN_BYTES: usize = crate::join::MAX_JOIN_CHAIN_BYTES;
const MAX_CHAIN_COLUMNS: usize = 128;
const MAX_CHAIN_HASH_BYTES: usize = MAX_CHAIN_BYTES;

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

fn write_group_header<W: Write>(
    query: &JoinSelect,
    selected: &[(usize, String, Option<usize>)],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metric = query.grouped_sum_position.unwrap_or(selected.len());
    for index in 0..=selected.len() {
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
        } else {
            let key = if index < metric { index } else { index - 1 };
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
    start: usize,
    rows: u64,
}

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
                        format!("{}.{}", column.relation, column.name)
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
    Ok(selected)
}

/// Retain only physical streams needed by the query's output, join keys,
/// optional NULL filter, and ORDER BY. Output rows still carry full-width
/// logical slots so existing join-key and projection indices remain valid.
fn required_physical_columns(
    query: &JoinSelect,
    relations: &[Relation],
    initial_step: &JoinStep,
    row_filters: Option<&row_filters::RowFilters>,
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
        if let Some(columns) = &query.columns {
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
    if let Some(crate::JoinAggregate::CountColumn(column)) = &query.aggregate {
        retain_absolute(column_index(relations, column)?);
    }
    if let Some((column, _)) = &query.where_null {
        retain_absolute(column_index(relations, column)?);
    }
    for (column, _, _) in &query.order_by {
        retain_absolute(column_index(relations, column)?);
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
    if kind != typed::base_type(&outer_type)
        || !matches!(
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
        )
    {
        return Err(invalid(
            "JOIN IN subquery requires matching integer key types",
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
        if value.is_empty()
            || !value
                .bytes()
                .enumerate()
                .all(|(i, byte)| byte.is_ascii_digit() || (i == 0 && byte == b'-'))
        {
            return Err(invalid("invalid JOIN IN subquery integer"));
        }
        if seen.insert(value.to_owned()) {
            if values.len() >= MAX_IN_SUBQUERY_KEYS {
                return Err(invalid("JOIN IN subquery supports at most 100000 keys"));
            }
            values.push(value.to_owned());
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
    execute_multi_join_inner(query, paths, None, None, output)
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
        Some((&plan.filters, inner_paths)),
        None,
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
    execute_multi_join_inner(plan.outer(), paths, None, Some((plan, inner)), output)
}

fn execute_multi_join_inner<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    exists_plan: Option<(&[crate::JoinExistsFilter], &[PathBuf])>,
    avg_plan: Option<(&crate::JoinCorrelatedAvg, &std::path::Path)>,
    output: &mut W,
) -> Result<(), ExecutionError> {
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
    if query.scalar_where.is_some()
        && (query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner))
    {
        return Err(invalid("JOIN boolean filters require INNER joins"));
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
        let names = metadata
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let types = metadata
            .columns
            .iter()
            .map(|column| column.data_type.clone())
            .collect::<Vec<_>>();
        let alias = match index {
            0 => query.left_alias.clone(),
            1 => query.right_alias.clone(),
            _ => query.steps[index - 2].alias.clone(),
        };
        width = width
            .checked_add(names.len())
            .ok_or_else(|| invalid("JOIN column count overflow"))?;
        if width > MAX_CHAIN_COLUMNS {
            return Err(invalid("JOIN supports at most 128 source columns"));
        }
        relations.push(Relation {
            alias,
            names,
            types,
            start: width - metadata.columns.len(),
            rows: metadata.rows,
        });
    }
    let exists = if let Some((filters, inner_paths)) = exists_plan {
        if query.grouped_count.is_none()
            || query.aggregate.is_some()
            || query.grouped_sum.is_some()
            || query.grouped_min.is_some()
            || query.grouped_distinct.is_some()
            || query.grouped_count_column.is_some()
            || !query.grouped_case_counts.is_empty()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        {
            return Err(invalid(
                "correlated JOIN EXISTS requires grouped INNER COUNT",
            ));
        }
        exists::bind(filters, inner_paths, &relations)?
    } else {
        Vec::new()
    };
    let avg = if let Some((plan, path)) = avg_plan {
        if query.scalar_formula.is_none()
            || query.grouped_count.is_some()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        {
            return Err(invalid("correlated AVG requires scalar INNER JOIN formula"));
        }
        Some(correlated_avg::bind(plan, path, &relations)?)
    } else {
        None
    };

    if !query.grouped_key_aliases.is_empty() {
        if query.grouped_count.is_none()
            || query.kind != JoinType::Inner
            || query.steps.iter().any(|step| step.kind != JoinType::Inner)
        {
            return Err(invalid("JOIN key aliases require a grouped INNER join"));
        }
        for (position, alias) in &query.grouped_key_aliases {
            if query
                .columns
                .as_ref()
                .is_none_or(|columns| *position >= columns.len())
                || relations.iter().any(|relation| {
                    relation
                        .names
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(alias))
                })
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
                    let column = JoinColumn {
                        relation: String::new(),
                        name: name.clone(),
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
    let formula = query
        .scalar_formula
        .as_ref()
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

    let row_filters = query
        .scalar_where
        .as_ref()
        .map(|branches| row_filters::RowFilters::bind(branches, &relations))
        .transpose()?;

    // Validate the complete output schema without materializing rows.
    if query.limit == Some(0)
        || ((query.aggregate.is_some()
            || query.scalar_sum.is_some()
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
            if !(column.is_count_order() && query.grouped_count.is_some()) {
                column_index(&relations, column)?;
            }
        }
        if query.scalar_sum.is_some() || query.scalar_formula.is_some() {
            write_scalar_sum(query, None, false, output)?;
        } else if let Some((_, header)) = &count_column {
            writeln!(output, "{header}")?;
        } else if query.aggregate == Some(crate::JoinAggregate::CountStar) {
            writeln!(
                output,
                "{}",
                query.aggregate_alias.as_deref().unwrap_or("count")
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
    // right-hand build table fits the shared chain memory budget.
    if query.kind == JoinType::Inner
        && query.steps.iter().all(|step| step.kind == JoinType::Inner)
        && (query.order_by.is_empty() || query.grouped_count.is_some())
        && query.aggregate.is_none()
    {
        return streaming::execute(
            query,
            paths,
            &relations,
            &initial_step,
            &comparisons,
            &column_comparisons,
            grouped_sum.as_ref(),
            formula.as_ref(),
            distinct_index,
            row_filters.as_ref(),
            &exists,
            avg.as_ref(),
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
    let required =
        required_physical_columns(query, &relations, &initial_step, row_filters.as_ref())?;
    let mut rows = read_projected_build_rows(
        &paths[0],
        &relations[0].names,
        &required[0],
        relations[0].rows,
        MAX_INPUT_ROWS,
        MAX_STREAM_BUILD_BYTES,
    )?;
    for (step_index, step) in std::iter::once(&initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let right_relation = &relations[step_index + 1];
        let left_width = right_relation.start;
        let (left_keys, right_keys) = step_keys(step, &relations[..=step_index + 1], left_width)?;
        // INNER and LEFT cannot create rows from an empty intermediate.
        // Keep the source row limit even when its payload is not needed.
        if rows.is_empty() && matches!(step.kind, JoinType::Inner | JoinType::Left) {
            if right_relation.rows > MAX_INPUT_ROWS {
                return Err(invalid(format!(
                    "JOIN supports at most {MAX_INPUT_ROWS} input rows per table"
                )));
            }
            continue;
        }
        let right_rows = read_projected_build_rows(
            &paths[step_index + 1],
            &right_relation.names,
            &required[step_index + 1],
            right_relation.rows,
            MAX_INPUT_ROWS,
            MAX_STREAM_BUILD_BYTES,
        )?;
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
                query.aggregate_alias.as_deref().unwrap_or("count")
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
                        if step_index != query.steps.len()
                            || (comparisons.iter().all(|(index, operator, literal)| {
                                typed::matches_comparison(&joined[*index], literal, *operator)
                            }) && column_comparisons.iter().all(|(left, operator, right)| {
                                typed::matches_comparison(
                                    &joined[*left],
                                    &joined[*right],
                                    *operator,
                                )
                            }) && row_filters
                                .as_ref()
                                .is_none_or(|filters| filters.passes(&joined)))
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
                push_visible_row(&mut next, joined, &mut retained, filter)?;
            }
        }
        if let Some(matched_right) = matched_right {
            for (right_index, matched) in matched_right.into_iter().enumerate() {
                if !matched {
                    let mut joined = vec![typed::Cell::Null; left_width];
                    joined.extend_from_slice(&right_rows[right_index]);
                    push_visible_row(&mut next, joined, &mut retained, filter)?;
                }
            }
        }
        rows = next;
    }

    if !query.order_by.is_empty() {
        let ordering_columns = query
            .order_by
            .iter()
            .map(|(column, descending, nulls_first)| {
                column_index(&relations, column).map(|index| (index, *descending, *nulls_first))
            })
            .collect::<Result<Vec<_>, _>>()?;
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
    if query.aggregate == Some(crate::JoinAggregate::CountStar) {
        writeln!(
            output,
            "{}",
            query.aggregate_alias.as_deref().unwrap_or("count")
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
    for row in rows
        .iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (position, (index, _, fallback)) in selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
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
