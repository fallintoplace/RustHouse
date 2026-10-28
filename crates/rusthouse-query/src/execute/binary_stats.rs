//! O(1) memory Welford/Chan covariance and correlation over typed fact rows.
//! Pairwise NULL elimination and compiled WHERE predicates share the existing
//! validated typed scanner; no rows are sorted or materialized.
use super::predicates::{
    compile_numbers_predicates, monotonic_range_indices, number_matches_all,
};
use super::typed;
use super::{ExecutionError, Number, number_at, write_header};
use crate::{BinaryStatFunction, Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

#[derive(Default, Clone, Copy)]
struct PairMoments {
    n: u64,
    mean_x: f64,
    mean_y: f64,
    m2_x: f64,
    m2_y: f64,
    covariance: f64,
}

impl PairMoments {
    fn observe(&mut self, x: f64, y: f64) -> Result<(), ExecutionError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(ExecutionError::Invalid(
                "correlation does not accept nonfinite numeric values".into(),
            ));
        }
        self.n = self
            .n
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("covariance row count overflow".into()))?;
        let n = self.n as f64;
        let delta_x = x - self.mean_x;
        let delta_y = y - self.mean_y;
        self.mean_x += delta_x / n;
        self.mean_y += delta_y / n;
        self.m2_x += delta_x * (x - self.mean_x);
        self.m2_y += delta_y * (y - self.mean_y);
        self.covariance += delta_x * (y - self.mean_y);
        Ok(())
    }

    /// Exact sufficient-state merge allows future parallel per-part reductions.
    fn merge(&mut self, rhs: &Self) -> Result<(), ExecutionError> {
        if rhs.n == 0 {
            return Ok(());
        }
        if self.n == 0 {
            *self = *rhs;
            return Ok(());
        }
        let combined = self
            .n
            .checked_add(rhs.n)
            .ok_or_else(|| ExecutionError::Invalid("covariance row count overflow".into()))?;
        let n1 = self.n as f64;
        let n2 = rhs.n as f64;
        let n = combined as f64;
        let dx = rhs.mean_x - self.mean_x;
        let dy = rhs.mean_y - self.mean_y;
        let factor = n1 * n2 / n;
        self.mean_x += dx * n2 / n;
        self.mean_y += dy * n2 / n;
        self.m2_x += rhs.m2_x + dx * dx * factor;
        self.m2_y += rhs.m2_y + dy * dy * factor;
        self.covariance += rhs.covariance + dx * dy * factor;
        self.n = combined;
        Ok(())
    }

    fn value(&self, function: BinaryStatFunction) -> Option<f64> {
        let result = match function {
            BinaryStatFunction::Corr if self.n >= 2 && self.m2_x > 0.0 && self.m2_y > 0.0 => {
                self.covariance / (self.m2_x * self.m2_y).sqrt()
            }
            BinaryStatFunction::CovarPop if self.n > 0 => self.covariance / self.n as f64,
            BinaryStatFunction::CovarSamp if self.n >= 2 => {
                self.covariance / (self.n - 1) as f64
            }
            _ => return None,
        };
        result.is_finite().then_some(result)
    }
}

fn number_as_float(number: Number) -> f64 {
    match number {
        Number::Unsigned(value) => value as f64,
        Number::Signed(value) => value as f64,
        Number::Float(value) => value,
    }
}

fn numeric_at(column: &TypedColumn, row: usize) -> Result<Option<f64>, ExecutionError> {
    match column {
        TypedColumn::Numeric(values) => Ok(Some(number_as_float(number_at(values, row)))),
        TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 => {
            Ok(Some(number_as_float(number_at(values, row))))
        }
        TypedColumn::NullableNumeric { .. } => Ok(None),
        _ => Err(ExecutionError::Invalid(
            "covariance requires numeric columns".into(),
        )),
    }
}

fn write_result<W: Write>(
    query: &Select,
    output: &mut W,
    function: BinaryStatFunction,
    x: &str,
    y: &str,
    value: Option<f64>,
) -> Result<(), ExecutionError> {
    write_header(query, output, &format!("{}({x},{y})", function.name()))?;
    if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
        match value {
            Some(number) => writeln!(output, "{number}")?,
            None => writeln!(output, "NULL")?,
        }
    }
    Ok(())
}

pub(super) fn execute_part<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::BinaryStat { function, x, y } = &query.projection else {
        unreachable!("binary statistic needs two projected inputs");
    };
    let metadata = inspect_query_source(part)?;
    for name in [x, y] {
        let data_type = metadata
            .columns
            .iter()
            .find(|column| column.name == *name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown statistic column: {name}")))?
            .data_type
            .as_str();
        let data_type = typed::base_type(data_type);
        if !(data_type.starts_with("UInt")
            || data_type.starts_with("Int")
            || data_type.starts_with("Float"))
        {
            return Err(ExecutionError::Invalid(
                "correlation and covariance require numeric columns".into(),
            ));
        }
    }
    if query.limit == Some(0) {
        return write_result(query, output, *function, x, y, None);
    }
    let mut names = vec![x.clone()];
    if !names.contains(y) {
        names.push(y.clone());
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !names.iter().any(|name| name == predicate.column()) {
            names.push(predicate.column().to_owned());
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<String, usize>>();
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<String, String>>();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let mut moments = PairMoments::default();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            if let (Some(x), Some(y)) = (
                numeric_at(&batch[indices[x]], row)?,
                numeric_at(&batch[indices[y]], row)?,
            ) {
                moments.observe(x, y)?;
            }
        }
    }
    write_result(query, output, *function, x, y, moments.value(*function))
}

pub(super) fn execute_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::BinaryStat { function, x, y } = &query.projection else {
        unreachable!("generated statistic requires binary statistic projection");
    };
    if x != "number" || y != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    if query.limit == Some(0) {
        return write_result(query, output, *function, x, y, None);
    }
    let conditions = compile_numbers_predicates(query)?;
    let range = if query.predicates.is_empty() && query.or_predicates.is_empty() {
        Some((0, rows))
    } else {
        monotonic_range_indices(&conditions, start, rows, step)
    };
    if let Some((lower, upper)) = range {
        let count = upper - lower;
        let result = match function {
            BinaryStatFunction::Corr if count >= 2 && step > 0 => Some(1.0),
            BinaryStatFunction::CovarPop if count > 0 => {
                let n = count as f64;
                let step = step as f64;
                Some(step * step * (n * n - 1.0) / 12.0)
            }
            BinaryStatFunction::CovarSamp if count >= 2 => {
                let n = count as f64;
                let step = step as f64;
                Some(step * step * n * (n + 1.0) / 12.0)
            }
            _ => None,
        };
        return write_result(query, output, *function, x, y, result);
    }
    if rows > 10_000_000 {
        return Err(ExecutionError::Invalid(
            "large generated covariance requires monotonic WHERE predicates".into(),
        ));
    }
    let mut moments = PairMoments::default();
    for index in 0..rows {
        let number = Number::Unsigned(
            u128::from(start) + u128::from(index) * u128::from(step),
        );
        if number_matches_all(number, &conditions) {
            let value = number_as_float(number);
            moments.observe(value, value)?;
        }
    }
    write_result(query, output, *function, x, y, moments.value(*function))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_correlation_and_negative_covariance() {
        let mut moments = PairMoments::default();
        for x in 0..200_000 {
            moments
                .observe(x as f64 + 1_000_000.0, 500_000.0 - 3.0 * x as f64)
                .unwrap();
        }
        assert!((moments.value(BinaryStatFunction::Corr).unwrap() + 1.0).abs() < 1e-12);
        assert!(moments.value(BinaryStatFunction::CovarPop).unwrap() < 0.0);
    }

    #[test]
    fn partition_merging_matches_global_moments() {
        let mut left = PairMoments::default();
        let mut right = PairMoments::default();
        let mut global = PairMoments::default();
        for index in 0..120_000 {
            let x = index as f64;
            let y = 4.0 * x + 11.0;
            global.observe(x, y).unwrap();
            if index % 2 == 0 {
                left.observe(x, y).unwrap();
            } else {
                right.observe(x, y).unwrap();
            }
        }
        left.merge(&right).unwrap();
        assert!((left.value(BinaryStatFunction::Corr).unwrap()
            - global.value(BinaryStatFunction::Corr).unwrap()).abs() < 1e-9);
        let relative = ((left.value(BinaryStatFunction::CovarPop).unwrap()
            / global.value(BinaryStatFunction::CovarPop).unwrap()) - 1.0).abs();
        assert!(relative < 1e-9, "{relative}");
    }

    #[test]
    fn undefined_and_null_population_values() {
        let mut moments = PairMoments::default();
        assert!(moments.value(BinaryStatFunction::Corr).is_none());
        assert!(moments.value(BinaryStatFunction::CovarPop).is_none());
        moments.observe(10.0, 20.0).unwrap();
        assert!(moments.value(BinaryStatFunction::Corr).is_none());
        assert_eq!(moments.value(BinaryStatFunction::CovarPop), Some(0.0));
        assert!(moments.value(BinaryStatFunction::CovarSamp).is_none());
    }
}
