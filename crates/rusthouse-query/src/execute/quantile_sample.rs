//! Bounded-memory analytical quantiles over typed numeric fact columns.
//! A deterministic 8192-element reservoir samples uniformly from arbitrarily
//! many non-NULL rows. Ranks are approximate beyond the reservoir capacity.
use super::predicates::{
    compile_numbers_predicates, monotonic_range_indices, number_matches_all,
};
use super::typed;
use super::{ExecutionError, Number, compare_numbers, number_at, write_header};
use crate::{Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

pub(super) const CAPACITY: usize = 8192;
const SEED: u64 = 0x7265736572766f69;

#[derive(Default)]
pub(super) struct Reservoir {
    seen: u64,
    values: Vec<Number>,
}

impl Reservoir {
    fn mix(mut x: u64) -> u64 {
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58476d1ce4e5b9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94d049bb133111eb);
        x ^ (x >> 31)
    }

    pub(super) fn observe(&mut self, value: Number) -> Result<(), ExecutionError> {
        self.seen = self
            .seen
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("quantile sample count overflow".into()))?;
        if self.values.len() < CAPACITY {
            self.values.push(value);
        } else {
            // Algorithm R reservoir sampling; no per-row allocations after the
            // initial sample fills. A deterministic mix makes tests repeatable.
            let ticket = Self::mix(self.seen ^ SEED) % self.seen;
            if ticket < CAPACITY as u64 {
                self.values[ticket as usize] = value;
            }
        }
        Ok(())
    }

    pub(super) fn quantile(&mut self, probability_ppm: u32) -> Option<Number> {
        if self.values.is_empty() {
            return None;
        }
        let p = f64::from(probability_ppm) / 1_000_000.0;
        let rank = ((self.values.len() - 1) as f64 * p).round() as usize;
        self.values
            .select_nth_unstable_by(rank, |left, right| compare_numbers(*left, *right));
        Some(self.values[rank])
    }
}

fn header(column: &str, probability_ppm: u32) -> String {
    format!(
        "quantile({})({column})",
        f64::from(probability_ppm) / 1_000_000.0
    )
}

fn write_result<W: Write>(
    query: &Select,
    output: &mut W,
    result: Option<Number>,
    column: &str,
    probability_ppm: u32,
) -> Result<(), ExecutionError> {
    write_header(query, output, &header(column, probability_ppm))?;
    if query.limit != Some(0) && query.offset.unwrap_or(0) == 0 {
        match result {
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
    let Projection::ApproxQuantile {
        column,
        probability_ppm,
    } = &query.projection else {
        unreachable!("quantile executor requires quantile projection");
    };
    let metadata = inspect_query_source(part)?;
    let source_type = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown quantile column: {column}")))?
        .data_type
        .as_str();
    if !source_type
        .strip_prefix("Nullable(")
        .unwrap_or(source_type)
        .starts_with("Float")
        && !source_type
            .strip_prefix("Nullable(")
            .unwrap_or(source_type)
            .starts_with("Int")
        && !source_type
            .strip_prefix("Nullable(")
            .unwrap_or(source_type)
            .starts_with("UInt")
    {
        return Err(ExecutionError::Invalid(
            "quantile() requires numeric columns".into(),
        ));
    }
    if query.limit == Some(0) {
        return write_result(query, output, None, column, *probability_ppm);
    }
    let mut names = vec![column.clone()];
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !names.iter().any(|name| name == predicate.column()) {
            names.push(predicate.column().to_owned());
        }
    }
    let indices: HashMap<String, usize> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let types: HashMap<String, String> = metadata
        .columns
        .iter()
        .map(|candidate| (candidate.name.clone(), candidate.data_type.clone()))
        .collect();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let mut sample = Reservoir::default();
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let index = indices[column];
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            let number = match &batch[index] {
                TypedColumn::Numeric(values) => Some(number_at(values, row)),
                TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 => {
                    Some(number_at(values, row))
                }
                TypedColumn::NullableNumeric { .. } => None,
                _ => {
                    return Err(ExecutionError::Invalid(
                        "quantile() requires numeric column batches".into(),
                    ));
                }
            };
            if let Some(value) = number {
                sample.observe(value)?;
            }
        }
    }
    write_result(query, output, sample.quantile(*probability_ppm), column, *probability_ppm)
}

pub(super) fn execute_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::ApproxQuantile {
        column,
        probability_ppm,
    } = &query.projection else {
        unreachable!("generated quantile requires quantile projection");
    };
    if column != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    if query.limit == Some(0) || query.offset.unwrap_or(0) > 0 {
        return write_result(query, output, None, column, *probability_ppm);
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let predicates = compile_numbers_predicates(query)?;
    let monotonic = if no_filters {
        Some((0, rows))
    } else {
        monotonic_range_indices(&predicates, start, rows, step)
    };
    if let Some((lower, upper)) = monotonic {
        let count = upper - lower;
        let result = if count == 0 {
            None
        } else {
            let p = f64::from(*probability_ppm) / 1_000_000.0;
            let rank = ((count - 1) as f64 * p).round() as u64;
            let position = u128::from(lower + rank);
            Some(Number::Unsigned(
                u128::from(start) + position * u128::from(step),
            ))
        };
        return write_result(query, output, result, column, *probability_ppm);
    }
    if rows > 10_000_000 {
        return Err(ExecutionError::Invalid(
            "large generated quantile requires monotonic WHERE predicates".into(),
        ));
    }
    let mut sample = Reservoir::default();
    for index in 0..rows {
        let number = Number::Unsigned(
            u128::from(start) + u128::from(index) * u128::from(step),
        );
        if number_matches_all(number, &predicates) {
            sample.observe(number)?;
        }
    }
    write_result(query, output, sample.quantile(*probability_ppm), column, *probability_ppm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_below_capacity_and_close_to_true_rank_above_capacity() {
        let mut small = Reservoir::default();
        for value in 0..1000 {
            small.observe(Number::Unsigned(value)).unwrap();
        }
        assert_eq!(small.quantile(500_000).unwrap().to_string(), "500");
        let mut large = Reservoir::default();
        for value in 0..200_000 {
            large.observe(Number::Unsigned(value)).unwrap();
        }
        let result = large.quantile(950_000).unwrap();
        let Number::Unsigned(value) = result else { panic!("expected integer") };
        assert!(value.abs_diff(190_000) < 4_000, "{value}");
        assert_eq!(large.values.len(), CAPACITY);
        assert_eq!(large.seen, 200_000);
    }

    #[test]
    fn empty_results_and_extreme_quantiles() {
        let mut empty = Reservoir::default();
        assert!(empty.quantile(500_000).is_none());
        let mut small = Reservoir::default();
        for number in [99, 2, 10] {
            small.observe(Number::Signed(number)).unwrap();
        }
        assert_eq!(small.quantile(0).unwrap().to_string(), "2");
        assert_eq!(small.quantile(1_000_000).unwrap().to_string(), "99");
    }
}
