//! Bounded two-key sums for a correlated join filter.
use super::*;
use crate::{JoinCorrelatedSum, JoinExistsKey, Operator};
use rusthouse_storage::TypedQueryReader;

const MAX_SUM_KEYS: usize = 100_000;

pub(super) struct BoundSum {
    pub(super) outer_keys: [usize; 2],
    pub(super) outer_value: usize,
    values: HashMap<(JoinExistsKey, JoinExistsKey), super::super::Number>,
}
fn numeric(kind: &str) -> bool {
    super::exists::integer(typed::base_type(kind))
        || matches!(typed::base_type(kind), "Float32" | "Float64")
}
impl BoundSum {
    pub(super) fn passes(&self, row: &[typed::Cell]) -> Result<bool, ExecutionError> {
        let Some(first) = super::exists::key(&row[self.outer_keys[0]])? else {
            return Ok(false);
        };
        let Some(second) = super::exists::key(&row[self.outer_keys[1]])? else {
            return Ok(false);
        };
        let Some(value) = self.values.get(&(first, second)) else {
            return Ok(false);
        };
        Ok(typed::matches_comparison(
            &row[self.outer_value],
            &typed::Cell::Numeric(*value),
            Operator::Gt,
        ))
    }
}

pub(super) fn bind(
    plan: &JoinCorrelatedSum,
    path: &std::path::Path,
    outer: &[Relation],
    skip_rows: bool,
) -> Result<BoundSum, ExecutionError> {
    let metadata = inspect_query_source(path)?;
    let inner = Relation {
        alias: plan.inner_alias.clone(),
        names: metadata
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect(),
        types: metadata
            .columns
            .iter()
            .map(|column| column.data_type.clone())
            .collect(),
        start: 0,
        rows: metadata.rows,
    };
    let relations = [inner];
    let inner_keys = [
        column_index(&relations, &plan.inner_keys[0])?,
        column_index(&relations, &plan.inner_keys[1])?,
    ];
    let inner_value = column_index(&relations, &plan.inner_value)?;
    let outer_keys = [
        column_index(outer, &plan.outer_keys[0])?,
        column_index(outer, &plan.outer_keys[1])?,
    ];
    let outer_value = column_index(outer, &plan.outer_value)?;
    for index in 0..2 {
        let kind = typed::base_type(column_type(&relations, inner_keys[index]));
        if !super::exists::integer(kind)
            || kind != typed::base_type(column_type(outer, outer_keys[index]))
        {
            return Err(invalid("correlated SUM requires matching integer keys"));
        }
    }
    if !numeric(column_type(&relations, inner_value)) || !numeric(column_type(outer, outer_value)) {
        return Err(invalid("correlated SUM requires numeric values"));
    }
    let factor = if plan.factor.contains(['.', 'e', 'E']) {
        let value = plan
            .factor
            .parse::<f64>()
            .map_err(|_| invalid("invalid correlated SUM factor"))?;
        if !value.is_finite() {
            return Err(invalid("invalid correlated SUM factor"));
        }
        super::super::multi_aggregate::BoundExpr::Float(value)
    } else {
        super::super::multi_aggregate::BoundExpr::Integer(
            plan.factor
                .parse::<i64>()
                .map_err(|_| invalid("invalid correlated SUM integer factor"))?,
        )
    };
    let filters = super::row_filters::RowFilters::bind(&plan.inner_filters, &relations)?;
    let mut projected = vec![inner_keys[0], inner_keys[1], inner_value];
    projected.extend_from_slice(&filters.indices);
    projected.sort_unstable();
    projected.dedup();
    let mut values = HashMap::new();
    if !skip_rows {
        let fields = projected
            .iter()
            .map(|&index| relations[0].names[index].clone())
            .collect::<Vec<_>>();
        let mut reader = TypedQueryReader::open(path, &fields)?;
        let mut rows = 0u64;
        let mut row = vec![typed::Cell::Null; relations[0].names.len()];
        while let Some(batch) = reader.next_batch()? {
            for index in 0..batch[0].len() {
                rows = rows
                    .checked_add(1)
                    .ok_or_else(|| invalid("correlated SUM row count overflow"))?;
                for (position, &column) in projected.iter().enumerate() {
                    row[column] = typed::cell(&batch[position], index);
                }
                if !filters.passes(&row) {
                    continue;
                }
                let Some(first) = super::exists::key(&row[inner_keys[0]])? else {
                    continue;
                };
                let Some(second) = super::exists::key(&row[inner_keys[1]])? else {
                    continue;
                };
                let value = match row[inner_value] {
                    typed::Cell::Null => continue,
                    typed::Cell::Numeric(value) => value,
                    _ => return Err(invalid("correlated SUM requires numeric values")),
                };
                let key = (first, second);
                if let Some(sum) = values.get_mut(&key) {
                    *sum = super::super::checked_aggregate_sum(*sum, value)?;
                } else {
                    if values.len() >= MAX_SUM_KEYS {
                        return Err(invalid("correlated SUM supports at most 100000 keys"));
                    }
                    values.insert(key, value);
                }
            }
        }
        if rows != metadata.rows {
            return Err(invalid(
                "correlated SUM input row count changed during scan",
            ));
        }
    }
    let factor_expr = super::super::multi_aggregate::BoundExpr::Multiply(
        Box::new(super::super::multi_aggregate::BoundExpr::Column(0)),
        Box::new(factor),
    );
    for value in values.values_mut() {
        *value = super::super::multi_aggregate::eval_row_arithmetic(
            &factor_expr,
            &[typed::Cell::Numeric(*value)],
        )?
        .ok_or_else(|| invalid("invalid correlated SUM result"))?;
    }
    Ok(BoundSum {
        outer_keys,
        outer_value,
        values,
    })
}
