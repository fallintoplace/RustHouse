//! Bounded memory numeric histogram over immutable analytical fact parts.
//! Two streaming passes find range then count equal-width bins. Memory scales
//! with bin count and decoded granule size, not SELECT cardinality.
use super::predicates::{
    compile_numbers_predicates, monotonic_range_indices, number_matches_all,
};
use super::typed;
use super::{ExecutionError, Number, number_at, write_header};
use crate::{Projection, Select};
use rusthouse_storage::{PartMetadata, TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MAX_GENERATED_HISTOGRAM_ROWS: u64 = 10_000_000;

#[derive(Clone, Copy)]
struct Bin {
    min: f64,
    max: f64,
    count: u64,
}

fn finite_number_as_f64(value: Number) -> Result<f64, ExecutionError> {
    let value = match value {
        Number::Unsigned(value) => value as f64,
        Number::Signed(value) => value as f64,
        Number::Float(value) => value,
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ExecutionError::Invalid(
            "histogram does not support NaN or infinite values".into(),
        ))
    }
}

fn bucket(value: f64, min: f64, max: f64, bins: usize) -> usize {
    if min == max {
        return 0;
    }
    let fraction = ((value - min) / (max - min)).clamp(0.0, 1.0);
    ((fraction * bins as f64) as usize).min(bins - 1)
}

fn to_bins(min: f64, max: f64, counts: &[u64]) -> Vec<Bin> {
    if min == max {
        return vec![Bin {
            min,
            max,
            count: counts.iter().copied().sum(),
        }];
    }
    let span = max - min;
    counts
        .iter()
        .enumerate()
        .map(|(index, &count)| Bin {
            min: min + span * (index as f64 / counts.len() as f64),
            max: if index + 1 == counts.len() {
                max
            } else {
                min + span * ((index + 1) as f64 / counts.len() as f64)
            },
            count,
        })
        .collect()
}

fn output_bins<W: Write>(
    query: &Select,
    output: &mut W,
    column: &str,
    bins: usize,
    result: &[Bin],
) -> Result<(), ExecutionError> {
    write_header(query, output, &format!("histogram({bins})({column})"))?;
    if query.limit == Some(0) || query.offset.unwrap_or(0) > 0 {
        return Ok(());
    }
    output.write_all(b"[")?;
    // Formatting directly into a writer emits several tiny writes per bin.
    // Reuse one short String so each result tuple is written in one call.
    let mut formatted = String::with_capacity(96);
    for (index, bin) in result.iter().enumerate() {
        formatted.clear();
        if index > 0 {
            formatted.push(',');
        }
        std::fmt::Write::write_fmt(
            &mut formatted,
            format_args!("({},{},{})", bin.min, bin.max, bin.count),
        ).expect("writing to String cannot fail");
        output.write_all(formatted.as_bytes())?;
    }
    output.write_all(b"]\n")?;
    Ok(())
}

/// Iterates a verified snapshot from the beginning, using identical compiled
/// filters in both passes. No values or rows are retained between passes.
fn visit_part(
    part: &Path,
    metadata: &PartMetadata,
    names: &[String],
    indices: &HashMap<String, usize>,
    column: &str,
    conditions: &[Vec<typed::Condition>],
    no_filters: bool,
    mut observe: impl FnMut(f64) -> Result<(), ExecutionError>,
) -> Result<(), ExecutionError> {
    let selected = indices[column];
    let mut reader = TypedQueryReader::open_with_metadata(part, names, metadata)?;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            match &batch[selected] {
                TypedColumn::Numeric(values) => {
                    observe(finite_number_as_f64(number_at(values, row))?)?
                }
                TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 => {
                    observe(finite_number_as_f64(number_at(values, row))?)?
                }
                TypedColumn::NullableNumeric { .. } => {}
                _ => return Err(ExecutionError::Invalid(
                    "histogram requires numerical columns".into(),
                )),
            }
        }
    }
    Ok(())
}

pub(super) fn execute_part<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::Histogram { column, bins } = &query.projection else {
        unreachable!("histogram executor requires histogram projection");
    };
    let metadata = inspect_query_source(part)?;
    let source_type = metadata
        .columns
        .iter()
        .find(|candidate| candidate.name == *column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown histogram column: {column}")))?
        .data_type
        .as_str();
    let base_type = typed::base_type(source_type);
    if !(base_type.starts_with("UInt")
        || base_type.starts_with("Int")
        || base_type.starts_with("Float"))
    {
        return Err(ExecutionError::Invalid(
            "histogram requires numeric column type".into(),
        ));
    }
    if query.limit == Some(0) {
        return output_bins(query, output, column, *bins, &[]);
    }
    let mut names = vec![column.clone()];
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !names.iter().any(|candidate| candidate == predicate.column()) {
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
        .map(|candidate| (candidate.name.clone(), candidate.data_type.clone()))
        .collect::<HashMap<String, String>>();
    // Both passes traverse the same immutable snapshot with identical
    // filters. Compile regex-like patterns and IN sets only once.
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut rows = 0u64;
    visit_part(part, &metadata, &names, &indices, column, &conditions, no_filters, |value| {
        min = min.min(value);
        max = max.max(value);
        rows = rows
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("histogram row count overflow".into()))?;
        Ok(())
    })?;
    if rows == 0 {
        return output_bins(query, output, column, *bins, &[]);
    }
    // The first pass established that every qualifying value is identical.
    // One exact bucket already contains all rows, so a second disk scan
    // cannot change the result.
    if min == max {
        return output_bins(query, output, column, *bins, &[Bin {
            min, max, count: rows,
        }]);
    }
    if !(max - min).is_finite() {
        return Err(ExecutionError::Invalid(
            "histogram numeric range exceeds Float64 precision".into(),
        ));
    }
    let mut counts = vec![0u64; *bins];
    visit_part(part, &metadata, &names, &indices, column, &conditions, no_filters, |value| {
        let index = bucket(value, min, max, *bins);
        counts[index] = counts[index]
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("histogram bin overflow".into()))?;
        Ok(())
    })?;
    if counts.iter().copied().sum::<u64>() != rows {
        return Err(ExecutionError::Invalid(
            "histogram input changed between passes".into(),
        ));
    }
    output_bins(query, output, column, *bins, &to_bins(min, max, &counts))
}

/// Find exact bucket boundaries using the same floating-point placement as
/// the physical two-pass histogram. Binary search keeps O(bins * log(rows))
/// work even when the generated source contains billions of values.
fn uniform_generated_bins(start: u64, lower: u64, count: u64, step: u64, bins: usize) -> Vec<Bin> {
    let value = |relative: u64| -> f64 {
        let index = u128::from(lower) + u128::from(relative);
        (u128::from(start) + index * u128::from(step)) as f64
    };
    let min = value(0);
    let max = value(count - 1);
    if min == max {
        return vec![Bin { min, max, count }];
    }
    let mut counts = vec![0u64; bins];
    let mut start_row = 0u64;
    for (bin, slot) in counts.iter_mut().enumerate() {
        if start_row == count {
            break;
        }
        let mut lo = start_row;
        let mut hi = count;
        while lo < hi {
            let middle = lo + (hi - lo) / 2;
            if bucket(value(middle), min, max, bins) <= bin {
                lo = middle + 1;
            } else {
                hi = middle;
            }
        }
        *slot = lo - start_row;
        start_row = lo;
    }
    // Match the stored-part executor: return equal-width boundaries and
    // retain empty buckets rather than replacing them with observed extrema.
    to_bins(min, max, &counts)
}

pub(super) fn execute_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::Histogram { column, bins } = &query.projection else {
        unreachable!("generated histogram requires histogram projection");
    };
    if column != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    if query.limit == Some(0) {
        return output_bins(query, output, column, *bins, &[]);
    }
    let conditions = compile_numbers_predicates(query)?;
    let range = if query.predicates.is_empty() && query.or_predicates.is_empty() {
        Some((0, rows))
    } else {
        monotonic_range_indices(&conditions, start, rows, step)
    };
    // A uniform arithmetic sequence has exact equal-width bucket counts.
    // Calculate billion-row histograms without scanning every generated row.
    if let Some((lower, upper)) = range {
        let count = upper - lower;
        if count == 0 {
            return output_bins(query, output, column, *bins, &[]);
        }
        let result = uniform_generated_bins(start, lower, count, step, *bins);
        return output_bins(query, output, column, *bins, &result);
    }
    if rows > MAX_GENERATED_HISTOGRAM_ROWS {
        return Err(ExecutionError::Invalid(
            "large generated histogram requires monotonic WHERE predicates".into(),
        ));
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for index in 0..rows {
        let number = u128::from(start) + u128::from(index) * u128::from(step);
        if number_matches_all(Number::Unsigned(number), &conditions) {
            let value = number as f64;
            min = min.min(value);
            max = max.max(value);
        }
    }
    if min == f64::INFINITY {
        return output_bins(query, output, column, *bins, &[]);
    }
    let mut counts = vec![0u64; *bins];
    for index in 0..rows {
        let number = u128::from(start) + u128::from(index) * u128::from(step);
        if number_matches_all(Number::Unsigned(number), &conditions) {
            counts[bucket(number as f64, min, max, *bins)] += 1;
        }
    }
    output_bins(query, output, column, *bins, &to_bins(min, max, &counts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_histogram_rows_with_one_output_write_per_bin() {
        use std::io;
        struct Counting {
            calls: usize,
            bytes: Vec<u8>,
        }
        impl Write for Counting {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let query = crate::parse_select("SELECT histogram(3)(number) FROM numbers(10)").unwrap();
        let bins = [
            Bin { min: 0.0, max: 3.0, count: 3 },
            Bin { min: 3.0, max: 6.0, count: 3 },
            Bin { min: 6.0, max: 9.0, count: 4 },
        ];
        let mut writer = Counting { calls: 0, bytes: Vec::new() };
        output_bins(&query, &mut writer, "number", 3, &bins).unwrap();
        assert!(writer.bytes.ends_with(b"[(0,3,3),(3,6,3),(6,9,4)]\n"));
        // Header, brackets and the three bins need a bounded number of writes.
        assert!(writer.calls <= 12, "too many writes: {}", writer.calls);
    }

    #[test]
    fn equal_width_boundaries_include_maximum_and_empty() {
        let mut counts = vec![0u64; 8];
        for index in 0..10_000 {
            counts[bucket(index as f64, 0.0, 9999.0, 8)] += 1;
        }
        assert_eq!(counts.iter().sum::<u64>(), 10_000);
        let mut uniform = vec![0u64; 4];
        for _ in 0..1000 {
            uniform[bucket(17.0, 17.0, 17.0, 4)] += 1;
        }
        assert_eq!(to_bins(17.0, 17.0, &uniform).len(), 1);
    }

    #[test]
    fn numerically_invalid_inputs_rejected() {
        assert!(finite_number_as_f64(Number::Float(f64::NAN)).is_err());
        assert!(finite_number_as_f64(Number::Float(f64::INFINITY)).is_err());
    }
}

#[cfg(test)]
mod generated_histogram_boundary_regression_tests {
    use super::*;

    #[test]
    fn fast_path_agrees_with_two_pass_bucket_assignment_for_uneven_ranges() {
        for count in 1..128u64 {
            for bins in 1..12usize {
                let mut expected = vec![0u64; bins];
                for value in 0..count {
                    expected[bucket(value as f64, 0.0, (count - 1) as f64, bins)] += 1;
                }
                let computed = uniform_generated_bins(0, 0, count, 1, bins);
                let expected = to_bins(0.0, (count - 1) as f64, &expected);
                assert_eq!(computed.iter().map(|bin| (bin.min, bin.max, bin.count))
                    .collect::<Vec<_>>(),
                    expected.iter().map(|bin| (bin.min, bin.max, bin.count))
                        .collect::<Vec<_>>(), "rows={count} bins={bins}");
                assert_eq!(computed.iter().map(|bin| bin.count).sum::<u64>(), count);
            }
        }
    }

    #[test]
    fn empty_bins_still_have_fixed_interval_boundaries() {
        let bins = uniform_generated_bins(10, 0, 2, 100, 8);
        assert_eq!(bins.len(), 8);
        assert_eq!(bins.iter().map(|b| b.count).sum::<u64>(), 2);
        assert_eq!(bins[0].min, 10.0);
        assert_eq!(bins[7].max, 110.0);
        assert!(bins.iter().any(|b| b.count == 0));
    }

    #[test]
    fn respects_floating_point_bucket_placement_for_large_start_and_step() {
        let start = 9_007_199_254_740_992u64;
        let count = 87u64;
        let step = 3u64;
        let bins = 11usize;
        let low = start as f64;
        let high = (u128::from(start) + u128::from(count - 1) * u128::from(step)) as f64;
        let mut expected = vec![0u64; bins];
        for i in 0..count {
            let value = (u128::from(start) + u128::from(i) * u128::from(step)) as f64;
            expected[bucket(value, low, high, bins)] += 1;
        }
        let actual = uniform_generated_bins(start, 0, count, step, bins);
        let expected = to_bins(low, high, &expected);
        assert_eq!(actual.iter().map(|bin| (bin.min, bin.max, bin.count)).collect::<Vec<_>>(),
            expected.iter().map(|bin| (bin.min, bin.max, bin.count)).collect::<Vec<_>>());
    }
}
