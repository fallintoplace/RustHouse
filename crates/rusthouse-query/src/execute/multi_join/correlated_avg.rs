//! Bounded per-key averages for a correlated join filter.
use super::*;
use crate::{JoinCorrelatedAvg, JoinExistsKey};
use rusthouse_storage::TypedQueryReader;

const MAX_AVG_KEYS: usize = 100_000;

pub(super) struct BoundAvg {
    pub(super) outer_key: usize,
    pub(super) outer_value: usize,
    values: HashMap<JoinExistsKey, f64>,
}

fn numeric(kind: &str) -> bool {
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
            | "Float32"
            | "Float64"
    )
}

impl BoundAvg {
    pub(super) fn passes(&self, row: &[typed::Cell]) -> Result<bool, ExecutionError> {
        let Some(key) = super::exists::key(&row[self.outer_key])? else {
            return Ok(false);
        };
        let Some(value) = self.values.get(&key) else {
            return Ok(false);
        };
        Ok(typed::matches_comparison(
            &row[self.outer_value],
            &typed::Cell::Numeric(super::super::Number::Float(*value)),
            crate::Operator::Lt,
        ))
    }
}

pub(super) fn bind(
    plan: &JoinCorrelatedAvg,
    path: &std::path::Path,
    outer: &[Relation],
) -> Result<BoundAvg, ExecutionError> {
    let metadata = inspect_query_source(path)?;
    let key = metadata
        .columns
        .iter()
        .position(|column| column.name == plan.inner_key)
        .ok_or_else(|| invalid("unknown correlated AVG inner key"))?;
    let value = metadata
        .columns
        .iter()
        .position(|column| column.name == plan.inner_value)
        .ok_or_else(|| invalid("unknown correlated AVG value"))?;
    let outer_key = column_index(outer, &plan.outer_key)?;
    let outer_value = column_index(outer, &plan.outer_value)?;
    let inner_key_kind = typed::base_type(&metadata.columns[key].data_type);
    if !super::exists::integer(inner_key_kind)
        || inner_key_kind != typed::base_type(column_type(outer, outer_key))
    {
        return Err(invalid("correlated AVG requires matching integer keys"));
    }
    if !numeric(&metadata.columns[value].data_type) || !numeric(column_type(outer, outer_value)) {
        return Err(invalid("correlated AVG requires numeric values"));
    }
    let factor = plan
        .factor
        .parse::<f64>()
        .map_err(|_| invalid("invalid correlated AVG factor"))?;
    if !factor.is_finite() {
        return Err(invalid("invalid correlated AVG factor"));
    }
    let fields = if key == value {
        vec![metadata.columns[key].name.clone()]
    } else {
        vec![
            metadata.columns[key].name.clone(),
            metadata.columns[value].name.clone(),
        ]
    };
    let value_position = usize::from(key != value);
    let mut reader = TypedQueryReader::open(path, &fields)?;
    let mut groups = HashMap::<JoinExistsKey, (super::super::Number, u64)>::new();
    let mut rows = 0u64;
    while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            rows = rows
                .checked_add(1)
                .ok_or_else(|| invalid("correlated AVG row count overflow"))?;
            let Some(key) = super::exists::key(&typed::cell(&batch[0], index))? else {
                continue;
            };
            let value = match typed::cell(&batch[value_position], index) {
                typed::Cell::Null => continue,
                typed::Cell::Numeric(value) => value,
                _ => return Err(invalid("correlated AVG requires numeric values")),
            };
            if let Some((sum, count)) = groups.get_mut(&key) {
                *sum = super::super::checked_aggregate_sum(*sum, value)?;
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| invalid("correlated AVG count overflow"))?;
            } else {
                if groups.len() >= MAX_AVG_KEYS {
                    return Err(invalid("correlated AVG supports at most 100000 keys"));
                }
                groups.insert(key, (value, 1));
            }
        }
    }
    if rows != metadata.rows {
        return Err(invalid(
            "correlated AVG input row count changed during scan",
        ));
    }
    let mut values = HashMap::with_capacity(groups.len());
    for (key, (sum, count)) in groups {
        let sum = match sum {
            super::super::Number::Unsigned(value) => value as f64,
            super::super::Number::Signed(value) => value as f64,
            super::super::Number::Float(value) => value,
        };
        let threshold = factor * (sum / count as f64);
        if !threshold.is_finite() {
            return Err(invalid("correlated AVG result is not finite"));
        }
        values.insert(key, threshold);
    }
    Ok(BoundAvg {
        outer_key,
        outer_value,
        values,
    })
}
