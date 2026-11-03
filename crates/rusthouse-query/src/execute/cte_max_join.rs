//! Bounded materialization for a grouped CTE joined at its maximum value.
use super::{ExecutionError, Number, execute, typed};
use crate::{CteMaxJoin, JoinExistsKey, ScalarExpr, Source};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;

const MAX_CTE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CTE_KEYS: usize = 100_000;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_OUTPUT_ROWS: usize = 131_072;

fn invalid(message: &str) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}
struct LimitedRows(Vec<u8>);
impl Write for LimitedRows {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_CTE_BYTES {
            return Err(io::Error::other("CTE result exceeds 16 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn integer(kind: &str) -> bool {
    matches!(
        typed::base_type(kind),
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
}

fn numeric(kind: &str) -> bool {
    integer(kind) || matches!(typed::base_type(kind), "Float32" | "Float64")
}

fn float_expression(
    expr: &ScalarExpr,
    columns: &[rusthouse_storage::Column],
) -> Result<bool, ExecutionError> {
    match expr {
        ScalarExpr::Column(name) => {
            let column = columns
                .iter()
                .find(|column| column.name == *name)
                .ok_or_else(|| invalid("unknown CTE SUM column"))?;
            if !numeric(&column.data_type) {
                return Err(invalid("CTE SUM requires numeric columns"));
            }
            Ok(matches!(
                typed::base_type(&column.data_type),
                "Float32" | "Float64"
            ))
        }
        ScalarExpr::Integer(_) => Ok(false),
        ScalarExpr::Decimal(_) => Ok(true),
        ScalarExpr::Add(a, b) | ScalarExpr::Subtract(a, b) | ScalarExpr::Multiply(a, b) => {
            Ok(float_expression(a, columns)? | float_expression(b, columns)?)
        }
        ScalarExpr::Divide(a, b) => {
            float_expression(a, columns)?;
            float_expression(b, columns)?;
            Ok(true)
        }
        _ => Err(invalid("CTE SUM requires numeric arithmetic")),
    }
}

fn key(value: &typed::Cell) -> Result<Option<JoinExistsKey>, ExecutionError> {
    match value {
        typed::Cell::Null => Ok(None),
        typed::Cell::Numeric(Number::Unsigned(value)) => Ok(Some(JoinExistsKey::Unsigned(*value))),
        typed::Cell::Numeric(Number::Signed(value)) => Ok(Some(JoinExistsKey::Signed(*value))),
        _ => Err(invalid("CTE JOIN requires integer keys")),
    }
}

/// Evaluate the grouped CTE and join its maximum rows while a read lock is held.
pub fn execute_cte_max_join<W: Write>(
    plan: &CteMaxJoin,
    inner_path: &Path,
    outer_path: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let inner = inspect_query_source(inner_path)?;
    let outer = inspect_query_source(outer_path)?;
    let inner_key = inner
        .columns
        .iter()
        .position(|column| column.name == plan.group_column)
        .ok_or_else(|| invalid("unknown CTE group key"))?;
    if inner.columns.iter().any(|column| {
        column.name != plan.group_column && column.name.eq_ignore_ascii_case(&plan.group_alias)
    }) {
        return Err(invalid("CTE group alias conflicts with an input column"));
    }
    let outer_key = outer
        .columns
        .iter()
        .position(|column| column.name == plan.outer_key)
        .ok_or_else(|| invalid("unknown CTE JOIN key"))?;
    let kind = typed::base_type(&inner.columns[inner_key].data_type);
    if !integer(kind) || kind != typed::base_type(&outer.columns[outer_key].data_type) {
        return Err(invalid("CTE JOIN requires matching integer keys"));
    }
    if outer.columns.iter().any(|column| {
        column.name.eq_ignore_ascii_case(&plan.group_alias)
            || column.name.eq_ignore_ascii_case(&plan.metric_alias)
    }) {
        return Err(invalid("ambiguous CTE JOIN column"));
    }
    let expression = plan
        .grouped
        .grouped_expressions
        .first()
        .and_then(Option::as_ref)
        .ok_or_else(|| invalid("CTE requires an expression SUM"))?;
    if !float_expression(expression, &inner.columns)? {
        return Err(invalid("CTE maximum requires a floating-point SUM"));
    }
    let mut projected = vec![outer_key];
    let mut selected = Vec::new();
    for name in &plan.columns {
        if name == &plan.metric_alias {
            selected.push(None);
        } else {
            let index = outer
                .columns
                .iter()
                .position(|column| column.name == *name)
                .ok_or_else(|| invalid("unknown CTE JOIN output column"))?;
            projected.push(index);
            selected.push(Some(index));
        }
    }
    let order = plan
        .order
        .as_ref()
        .map(|(name, desc, nulls)| {
            let index = outer
                .columns
                .iter()
                .position(|column| column.name == *name)
                .ok_or_else(|| invalid("unknown CTE JOIN ORDER BY column"))?;
            projected.push(index);
            Ok::<_, ExecutionError>((index, *desc, *nulls))
        })
        .transpose()?;
    projected.sort_unstable();
    projected.dedup();
    let position = |index: usize| {
        projected
            .binary_search(&index)
            .expect("retained CTE JOIN column")
    };
    let mut grouped = plan.grouped.clone();
    grouped.source = Some(Source::Table("part".into()));
    if plan.limit == Some(0) {
        grouped.limit = Some(0);
    }
    let mut intermediate = LimitedRows(Vec::new());
    execute(&grouped, Some(inner_path), &mut intermediate)?;
    if plan.limit == Some(0) {
        writeln!(output, "{}", plan.columns.join("\t"))?;
        return Ok(());
    }
    let text = std::str::from_utf8(&intermediate.0).map_err(|_| invalid("invalid CTE output"))?;
    let mut lines = text.lines();
    lines.next().ok_or_else(|| invalid("missing CTE header"))?;
    let mut values = HashMap::new();
    let mut maximum: Option<f64> = None;
    let mut count = 0usize;
    for line in lines {
        count += 1;
        if count > MAX_CTE_KEYS {
            return Err(invalid("CTE supports at most 100000 groups"));
        }
        let (raw_key, raw_value) = line
            .split_once('\t')
            .ok_or_else(|| invalid("invalid CTE row"))?;
        if raw_value == "\\N" || raw_value == "NULL" {
            continue;
        }
        let value = raw_value
            .parse::<f64>()
            .map_err(|_| invalid("invalid CTE SUM"))?;
        if !value.is_finite() {
            return Err(invalid("non-finite CTE SUM"));
        }
        maximum = Some(maximum.map_or(value, |previous| previous.max(value)));
        if raw_key == "\\N" {
            continue;
        }
        let key = if kind.starts_with("UInt") {
            JoinExistsKey::Unsigned(raw_key.parse().map_err(|_| invalid("invalid CTE key"))?)
        } else {
            JoinExistsKey::Signed(raw_key.parse().map_err(|_| invalid("invalid CTE key"))?)
        };
        if values.insert(key, value).is_some() {
            return Err(invalid("duplicate CTE group key"));
        }
    }
    let names = projected
        .iter()
        .map(|&index| outer.columns[index].name.clone())
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(outer_path, &names)?;
    let mut rows = Vec::<(Vec<typed::Cell>, f64)>::new();
    let mut retained = 0usize;
    let mut scanned = 0u64;
    while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            scanned = scanned
                .checked_add(1)
                .ok_or_else(|| invalid("CTE JOIN input row count overflow"))?;
            let Some(key) = key(&typed::cell(&batch[position(outer_key)], index))? else {
                continue;
            };
            let Some(&metric) = values.get(&key) else {
                continue;
            };
            if Some(metric) != maximum {
                continue;
            }
            if rows.len() >= MAX_OUTPUT_ROWS {
                return Err(invalid("CTE JOIN supports at most 131072 matched rows"));
            }
            let cells = batch
                .iter()
                .map(|column| typed::cell(column, index))
                .collect::<Vec<_>>();
            let bytes = cells.iter().try_fold(
                cells
                    .len()
                    .saturating_mul(std::mem::size_of::<typed::Cell>()),
                |total, cell| {
                    let extra = match cell {
                        typed::Cell::Text(bytes) => bytes.len(),
                        typed::Cell::Uuid(value) => value.len(),
                        _ => 0,
                    };
                    total
                        .checked_add(extra)
                        .ok_or_else(|| invalid("CTE JOIN output memory overflow"))
                },
            )?;
            retained = retained
                .checked_add(bytes)
                .filter(|&bytes| bytes <= MAX_OUTPUT_BYTES)
                .ok_or_else(|| invalid("CTE JOIN output exceeds 32 MiB"))?;
            rows.push((cells, metric));
        }
    }
    if scanned != outer.rows {
        return Err(invalid("CTE JOIN input row count changed during scan"));
    }
    if let Some((index, descending, nulls)) = order {
        rows.sort_by(|left, right| {
            typed::compare_order_cells(
                &left.0[position(index)],
                &right.0[position(index)],
                descending,
                nulls,
            )
        });
    }
    writeln!(output, "{}", plan.columns.join("\t"))?;
    for (cells, metric) in rows
        .into_iter()
        .skip(plan.offset)
        .take(plan.limit.unwrap_or(usize::MAX))
    {
        for (column, source) in selected.iter().enumerate() {
            if column > 0 {
                output.write_all(b"\t")?;
            }
            if let Some(index) = source {
                typed::write_typed_value(
                    output,
                    &cells[position(*index)],
                    &outer.columns[*index].data_type,
                )?;
            } else {
                write!(output, "{metric}")?;
            }
        }
        writeln!(output)?;
    }
    Ok(())
}
