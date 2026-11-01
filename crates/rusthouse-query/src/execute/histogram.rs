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

#[derive(Clone, Copy)]
struct BucketLayout {
    min: f64,
    max: f64,
    span: f64,
    bins: usize,
    bins_float: f64,
}

impl BucketLayout {
    fn new(min: f64, max: f64, bins: usize) -> Self {
        Self { min, max, span: max - min, bins, bins_float: bins as f64 }
    }

    fn bucket(self, value: f64) -> usize {
        if self.min == self.max {
            return 0;
        }
        let fraction = if self.span.is_finite() {
            (value - self.min) / self.span
        } else {
            (value * 0.5 - self.min * 0.5) / (self.max * 0.5 - self.min * 0.5)
        }
        .clamp(0.0, 1.0);
        ((fraction * self.bins_float) as usize).min(self.bins - 1)
    }
}

#[cfg(test)]
fn bucket(value: f64, min: f64, max: f64, bins: usize) -> usize {
    BucketLayout::new(min, max, bins).bucket(value)
}

// Derive floating-point bin boundaries lazily from compact counts instead
// of retaining an additional vector of Bin descriptors before output.
fn bins_iter(min: f64, max: f64, counts: &[u64]) -> impl Iterator<Item = Bin> + '_ {
    let span = max - min;
    let bin_count = counts.len() as f64;
    counts.iter().enumerate().map(move |(index, &count)| Bin {
        min: if span.is_finite() {
            min + span * (index as f64 / bin_count)
        } else {
            let t = index as f64 / bin_count;
            min * (1.0 - t) + max * t
        },
        max: if index + 1 == counts.len() {
            max
        } else if span.is_finite() {
            min + span * ((index + 1) as f64 / bin_count)
        } else {
            let t = (index + 1) as f64 / bin_count;
            min * (1.0 - t) + max * t
        },
        count,
    })
}

#[cfg(test)]
fn to_bins(min: f64, max: f64, counts: &[u64]) -> Vec<Bin> {
    if min == max {
        return vec![Bin {
            min, max, count: counts.iter().copied().sum(),
        }];
    }
    bins_iter(min, max, counts).collect()
}

fn output_bins<W: Write>(
    query: &Select,
    output: &mut W,
    column: &str,
    bins: usize,
    result: impl IntoIterator<Item = Bin>,
) -> Result<(), ExecutionError> {
    write_header(query, output, &format!("histogram({bins})({column})"))?;
    if query.limit == Some(0) || query.offset.unwrap_or(0) > 0 {
        return Ok(());
    }
    // Limit output calls by grouping histogram tuples into 64 KiB chunks.
    const OUTPUT_CHUNK_BYTES: usize = 64 * 1024;
    let mut formatted = String::with_capacity(OUTPUT_CHUNK_BYTES);
    formatted.push('[');
    for (index, bin) in result.into_iter().enumerate() {
        if index > 0 {
            formatted.push(',');
        }
        std::fmt::Write::write_fmt(
            &mut formatted,
            format_args!("({},{},{})", bin.min, bin.max, bin.count),
        ).expect("writing to String cannot fail");
        if formatted.len() >= OUTPUT_CHUNK_BYTES {
            output.write_all(formatted.as_bytes())?;
            formatted.clear();
        }
    }
    formatted.push_str("]\n");
    output.write_all(formatted.as_bytes())?;
    Ok(())
}

/// Render histogram counts without an intermediate floating-point bin vector.
fn output_counts<W: Write>(
    query: &Select,
    output: &mut W,
    column: &str,
    bins: usize,
    min: f64,
    max: f64,
    counts: &[u64],
) -> Result<(), ExecutionError> {
    if min == max {
        return output_bins(query, output, column, bins, std::iter::once(Bin {
            min, max, count: counts.iter().copied().sum(),
        }));
    }
    output_bins(query, output, column, bins, bins_iter(min, max, counts))
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
        // The physical column variant stays constant throughout one batch.
        // Specialize the row loop once instead of matching on every row.
        let rows = batch[0].len();
        match &batch[selected] {
            TypedColumn::Numeric(values) => {
                for row in 0..rows {
                    if no_filters || typed::passes(&batch, row, conditions) {
                        observe(finite_number_as_f64(number_at(values, row))?)?;
                    }
                }
            }
            TypedColumn::NullableNumeric { values, nulls } => {
                for row in 0..rows {
                    if nulls[row] == 0 && (no_filters || typed::passes(&batch, row, conditions)) {
                        observe(finite_number_as_f64(number_at(values, row))?)?;
                    }
                }
            }
            _ => return Err(ExecutionError::Invalid(
                "histogram requires numerical columns".into(),
            )),
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
        return output_bins(query, output, column, *bins, std::iter::empty());
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
        return output_bins(query, output, column, *bins, std::iter::empty());
    }
    // The first pass established that every qualifying value is identical.
    // One exact bucket already contains all rows, so a second disk scan
    // cannot change the result.
    if min == max {
        return output_bins(query, output, column, *bins, std::iter::once(Bin {
            min, max, count: rows,
        }));
    }
    // Both BucketLayout and to_bins use overflow-safe interpolation for
    // finite extrema whose difference is not representable as Float64.
    // The inputs themselves were validated by finite_number_as_f64.
    let mut counts = vec![0u64; *bins];
    let layout = BucketLayout::new(min, max, *bins);
    visit_part(part, &metadata, &names, &indices, column, &conditions, no_filters, |value| {
        let index = layout.bucket(value);
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
    output_counts(query, output, column, *bins, min, max, &counts)
}

/// Find exact bucket boundaries using the same floating-point placement as
/// the physical two-pass histogram. Binary search keeps O(bins * log(rows))
/// work even when the generated source contains billions of values.
fn uniform_generated_counts(start: u64, lower: u64, count: u64, step: u64, bins: usize) -> (f64, f64, Vec<u64>) {
    let value = |relative: u64| -> f64 {
        let index = u128::from(lower) + u128::from(relative);
        (u128::from(start) + index * u128::from(step)) as f64
    };
    let min = value(0);
    let max = value(count - 1);
    if min == max {
        return (min, max, vec![count]);
    }
    let mut counts = vec![0u64; bins];
    let layout = BucketLayout::new(min, max, bins);
    // With many output bins and few generated values, a binary search for
    // every bin is much more work than visiting each value once. Both paths
    // use the exact same float bucket layout and retain O(bins) memory.
    if count <= u64::try_from(bins).unwrap_or(u64::MAX).saturating_mul(4) {
        for relative in 0..count {
            counts[layout.bucket(value(relative))] += 1;
        }
        return (min, max, counts);
    }
    let mut start_row = 0u64;
    for (bin, slot) in counts.iter_mut().enumerate() {
        if start_row == count {
            break;
        }
        let mut lo = start_row;
        let mut hi = count;
        while lo < hi {
            let middle = lo + (hi - lo) / 2;
            if layout.bucket(value(middle)) <= bin {
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
    (min, max, counts)
}

#[cfg(test)]
fn uniform_generated_bins(start: u64, lower: u64, count: u64, step: u64, bins: usize) -> Vec<Bin> {
    let (min, max, counts) = uniform_generated_counts(start, lower, count, step, bins);
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
        return output_bins(query, output, column, *bins, std::iter::empty());
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
            return output_bins(query, output, column, *bins, std::iter::empty());
        }
        let (min, max, counts) = uniform_generated_counts(start, lower, count, step, *bins);
        return output_counts(query, output, column, *bins, min, max, &counts);
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
        return output_bins(query, output, column, *bins, std::iter::empty());
    }
    let mut counts = vec![0u64; *bins];
    let layout = BucketLayout::new(min, max, *bins);
    for index in 0..rows {
        let number = u128::from(start) + u128::from(index) * u128::from(step);
        if number_matches_all(Number::Unsigned(number), &conditions) {
            counts[layout.bucket(number as f64)] += 1;
        }
    }
    output_counts(query, output, column, *bins, min, max, &counts)
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
        output_bins(&query, &mut writer, "number", 3, bins.into_iter()).unwrap();
        assert!(writer.bytes.ends_with(b"[(0,3,3),(3,6,3),(6,9,4)]\n"));
        // Header, brackets and the three bins need a bounded number of writes.
        assert!(writer.calls <= 12, "too many writes: {}", writer.calls);
    }

    #[test]
    fn many_histogram_bins_stream_in_bounded_batches() {
        use std::io;
        struct Counted { writes: usize, max: usize, bytes: Vec<u8> }
        impl Write for Counted {
            fn write(&mut self, input: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.max = self.max.max(input.len());
                self.bytes.extend_from_slice(input);
                Ok(input.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        // The SQL grammar caps queries at 256 bins. Stress the private
        // output renderer with synthetic bins to exercise its chunk flush.
        let query = crate::parse_select("SELECT histogram(256)(number) FROM numbers(10000)").unwrap();
        let bins = (0..10_000).map(|value| Bin {
            min: value as f64,
            max: value as f64 + 1.0,
            count: 1,
        }).collect::<Vec<_>>();
        let mut sink = Counted { writes: 0, max: 0, bytes: Vec::new() };
        output_bins(&query, &mut sink, "number", 10_000, &bins).unwrap();
        assert!(sink.writes < 10, "one write per bin regressed: {}", sink.writes);
        assert!(sink.max < 65_600, "output batch grew without bound");
        let opening = sink.bytes.iter().position(|&byte| byte == b'[').unwrap();
        assert_eq!(sink.bytes[opening..].iter().filter(|&&byte| byte == b'(').count(), 10_000);
        assert!(sink.bytes.ends_with(b"(9999,10000,1)]\n"));
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

#[cfg(test)]
mod histogram_bucket_layout_tests {
    use super::*;

    #[test]
    fn prepared_bucket_layout_keeps_endpoints_and_range_boundaries() {
        for (min, max) in [(0.0, 1024.0), (-20.0, 20.0), (-1e308, 1e308), (17.0, 17.0)] {
            for bins in [1usize, 2, 4, 16, 128] {
                let prepared = BucketLayout::new(min, max, bins);
                for value in [min, min / 2.0, 0.0, max / 2.0, max] {
                    assert_eq!(prepared.bucket(value), bucket(value, min, max, bins));
                }
                assert!(prepared.bucket(max) < bins);
            }
        }
    }
}

#[cfg(test)]
mod finite_extreme_histogram_tests {
    use super::*;

    #[test]
    fn preserves_bins_when_finite_data_range_overflows_f64() {
        let min = -1e308;
        let max = 1e308;
        assert_eq!(bucket(min, min, max, 4), 0);
        assert_eq!(bucket(0.0, min, max, 4), 2);
        assert_eq!(bucket(max, min, max, 4), 3);
        let bins = to_bins(min, max, &[1, 1, 1, 1]);
        assert_eq!(bins.len(), 4);
        assert_eq!(bins.first().unwrap().min, min);
        assert_eq!(bins.last().unwrap().max, max);
        assert!(bins.iter().all(|bin| bin.min.is_finite() && bin.max.is_finite()));
        assert!(bins.windows(2).all(|pair| pair[0].max <= pair[1].min));
    }
}

#[cfg(test)]
mod wide_range_execution_tests {
    use super::*;
    use rusthouse_storage::{Column, write_wide_part};

    #[test]
    fn physical_histogram_accepts_finite_extrema_with_infinite_difference() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-histogram-extrema-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let columns = [Column { name: "value".into(), data_type: "Float64".into() }];
        let rows = [
            vec![Some("-1e308".into())],
            vec![Some("-5e307".into())],
            vec![Some("0".into())],
            vec![Some("5e307".into())],
            vec![Some("1e308".into())],
        ];
        write_wide_part(&root, &columns, &rows).unwrap();
        let query = crate::parse_select("SELECT histogram(4)(value) FROM part").unwrap();
        let mut output = Vec::new();
        execute_part(&query, &root, &mut output).unwrap();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("histogram(4)(value)"));
        assert!(rendered.ends_with(")]\n"));
        assert!(!rendered.to_ascii_lowercase().contains("inf"));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod sparse_generated_histogram_tests {
    use super::*;

    #[test]
    fn large_bin_counts_with_sparse_generated_values_are_exact() {
        for (start, lower, count, step, bins) in [
            (0u64, 0u64, 1u64, 1u64, 1024usize),
            (123, 3, 17, 5, 4096),
            (9_007_199_254_740_992, 11, 127, 3, 1024),
            (0, 0, 256, 7, 64),
            (1, 0, 400, 1, 100),
        ] {
            let actual = uniform_generated_bins(start, lower, count, step, bins);
            let value = |relative: u64| {
                (u128::from(start) + u128::from(lower + relative) * u128::from(step)) as f64
            };
            let min = value(0);
            let max = value(count - 1);
            let layout = BucketLayout::new(min, max, bins);
            let mut expected = vec![0u64; bins];
            for relative in 0..count {
                expected[layout.bucket(value(relative))] += 1;
            }
            let expected = to_bins(min, max, &expected);
            assert_eq!(
                actual.iter().map(|bin| (bin.min, bin.max, bin.count)).collect::<Vec<_>>(),
                expected.iter().map(|bin| (bin.min, bin.max, bin.count)).collect::<Vec<_>>(),
                "count={count} bins={bins}",
            );
        }
    }
}

#[cfg(test)]
mod streaming_histogram_bin_tests {
    use super::*;

    #[test]
    fn lazy_bins_match_materialized_reference_for_normal_and_extreme_ranges() {
        for (min, max, counts) in [
            (0.0, 100.0, vec![1u64, 2, 3, 4, 5, 6, 7]),
            (-f64::MAX, f64::MAX, vec![3, 0, 1, 0, 5]),
            (1e100, 1e101, vec![1, 9, 4, 0]),
            (13.0, 13.0, vec![2, 0, 5]),
        ] {
            let query = crate::parse_select(
                "SELECT histogram(7)(number) FROM numbers(10)"
            ).unwrap();
            let mut eager = Vec::new();
            output_bins(&query, &mut eager, "number", 7,
                to_bins(min, max, &counts).into_iter()).unwrap();
            let mut streamed = Vec::new();
            output_counts(&query, &mut streamed, "number", 7,
                min, max, &counts).unwrap();
            assert_eq!(streamed, eager, "min={min} max={max}");
        }
    }

    #[test]
    fn million_generated_rows_stream_same_bin_counts_as_reference() {
        let (min, max, counts) = uniform_generated_counts(0, 0, 1_000_000, 1, 256);
        let eager = uniform_generated_bins(0, 0, 1_000_000, 1, 256);
        assert_eq!(bins_iter(min, max, &counts).map(|bin|
            (bin.min, bin.max, bin.count)).collect::<Vec<_>>(),
            eager.iter().map(|bin| (bin.min, bin.max, bin.count))
                .collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod batched_histogram_column_dispatch_tests {
    use super::*;
    use rusthouse_storage::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn nullable_stored_histogram_ignores_null_rows() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-batched-histogram-{}-{}", std::process::id(), line!(),
        ));
        let _ = fs::remove_dir_all(&path);
        let columns = [Column { name: "value".into(), data_type: "Nullable(UInt64)".into() }];
        let rows = [
            vec![None], vec![Some("7".to_string())], vec![Some("7".to_string())],
            vec![None], vec![Some("7".to_string())],
        ];
        write_wide_part(&path, &columns, &rows).unwrap();
        let query = crate::parse_select("SELECT histogram(8)(value) FROM part").unwrap();
        let mut output = Vec::new();
        crate::execute(&query, Some(&path), &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "histogram(8)(value)\n[(7,7,3)]\n");
        fs::remove_dir_all(path).unwrap();
    }
}
