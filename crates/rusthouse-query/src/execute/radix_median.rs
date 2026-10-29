//! Bounded-memory exact median of physical integer columns.
//! One byte of the key is selected per immutable scan. Each pass uses only
//! 2 * 256 counters regardless of input cardinality.
use super::{ExecutionError, batch_matches, count_batch_matches, typed};
use super::predicates::CompiledPredicate;
use rusthouse_storage::{NumericColumn, NumericQueryReader, PartMetadata, TypedColumn, TypedQueryReader};
use std::path::Path;

fn width_and_sign(kind: &str) -> Option<(usize, bool)> {
    Some(match kind {
        "UInt8" => (1, false), "UInt16" => (2, false),
        "UInt32" => (4, false), "UInt64" => (8, false),
        "UInt128" => (16, false),
        "Int8" => (1, true), "Int16" => (2, true),
        "Int32" => (4, true), "Int64" => (8, true),
        "Int128" => (16, true),
        _ => return None,
    })
}

pub(super) fn supports_type(kind: &str) -> bool {
    width_and_sign(kind).is_some()
}

fn low_mask(bits: usize) -> u128 {
    if bits == 128 { u128::MAX } else { (1_u128 << bits) - 1 }
}

fn sorted_key_unsigned(value: u128) -> u128 { value }

fn sorted_key_signed(value: i128, bits: usize) -> u128 {
    ((value as u128) & low_mask(bits)) ^ (1_u128 << (bits - 1))
}

fn render_key(key: u128, bits: usize, signed: bool) -> String {
    if signed {
        signed_key_value(key, bits).to_string()
    } else {
        key.to_string()
    }
}

fn signed_key_value(key: u128, bits: usize) -> i128 {
    let sign = 1_u128 << (bits - 1);
    let raw = key ^ sign;
    let extended = if raw & sign != 0 { raw | !low_mask(bits) } else { raw };
    extended as i128
}

/// Exact median of two integers, including UInt128/Int128 extrema and half
/// values. Converting the operands to f64 first loses integer precision.
fn render_middle_pair(low_key: u128, high_key: u128, bits: usize, signed: bool) -> String {
    if signed {
        let low = signed_key_value(low_key, bits);
        let high = signed_key_value(high_key, bits);
        let remainder = low.rem_euclid(2) + high.rem_euclid(2);
        let floor = low.div_euclid(2) + high.div_euclid(2) + remainder / 2;
        if remainder % 2 == 0 {
            floor.to_string()
        } else if floor < 0 {
            format!("-{}.5", (floor + 1).unsigned_abs())
        } else {
            format!("{floor}.5")
        }
    } else {
        let remainder = (low_key & 1) + (high_key & 1);
        let floor = (low_key >> 1) + (high_key >> 1) + remainder / 2;
        if remainder % 2 == 0 { floor.to_string() } else { format!("{floor}.5") }
    }
}

/// Add the native integer batch directly to two radix histograms. Nullable
/// rows are skipped before key encoding, so NULLs never affect rank.
fn count_column(
    column: &NumericColumn,
    nulls: Option<&[u8]>,
    included: Option<&[bool]>,
    bits: usize,
    shift: usize,
    prefix_mask: u128,
    prefixes: &[u128; 2],
    histogram_sides: usize,
    histogram: &mut [[u64; 256]; 2],
) -> Result<(), ExecutionError> {
    let mut visit = |value: u128, row: usize| {
        if nulls.is_some_and(|map| map[row] != 0)
            || included.is_some_and(|mask| !mask[row])
        {
            return;
        }
        for side in 0..histogram_sides {
            if value & prefix_mask != prefixes[side] & prefix_mask {
                continue;
            }
            histogram[side][((value >> shift) & 255) as usize] += 1;
        }
    };
    macro_rules! unsigned {
        ($values:expr) => {
            for (index, &value) in $values.iter().enumerate() {
                visit(sorted_key_unsigned(u128::from(value)), index);
            }
        };
    }
    macro_rules! signed {
        ($values:expr) => {
            for (index, &value) in $values.iter().enumerate() {
                visit(sorted_key_signed(i128::from(value), bits), index);
            }
        };
    }
    match column {
        NumericColumn::UInt8(values) => unsigned!(values),
        NumericColumn::UInt16(values) => unsigned!(values),
        NumericColumn::UInt32(values) => unsigned!(values),
        NumericColumn::UInt64(values) => unsigned!(values),
        NumericColumn::UInt128(values) => unsigned!(values),
        NumericColumn::Int8(values) => signed!(values),
        NumericColumn::Int16(values) => signed!(values),
        NumericColumn::Int32(values) => signed!(values),
        NumericColumn::Int64(values) => signed!(values),
        NumericColumn::Int128(values) => signed!(values),
        _ => return Err(ExecutionError::Invalid(
            "radix median requires a native integer column".into()
        )),
    }
    Ok(())
}

/// Exact order statistics from bounded 256-bucket histograms. Each pass reads
/// immutable parts sequentially and retains no values from previous passes.
fn median_inner(
    part: &Path,
    column: &str,
    metadata: &PartMetadata,
    n: u64,
    nullable: bool,
    filtered: Option<(&[String], &[Vec<CompiledPredicate>])>,
    typed_filtered: Option<(&[String], &[Vec<typed::Condition>])>,
) -> Result<Option<String>, ExecutionError> {
    let kind = metadata.columns.iter().find(|item| item.name == column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown column: {column}")))?;
    let base = if nullable {
        kind.data_type.strip_prefix("Nullable(")
            .and_then(|kind| kind.strip_suffix(')'))
            .ok_or_else(|| ExecutionError::Invalid("expected Nullable column".into()))?
    } else {
        &kind.data_type
    };
    let (bytes, signed) = width_and_sign(base)
        .ok_or_else(|| ExecutionError::Invalid("radix median requires integer values".into()))?;
    if n == 0 { return Ok(None); }
    let bits = bytes * 8;
    let mut keys = [0_u128; 2];
    let mut ranks = [(n - 1) / 2, n / 2];
    let names = [column.to_owned()];
    for byte in (0..bytes).rev() {
        let shift = byte * 8;
        let prefix_mask = if shift + 8 >= 128 {
            0
        } else {
            u128::MAX << (shift + 8)
        };
        let mut histogram = [[0_u64; 256]; 2];
        let shared_prefix = keys[0] & prefix_mask == keys[1] & prefix_mask;
        let histogram_sides = if shared_prefix { 1 } else { 2 };
        if nullable {
            let columns = typed_filtered.map_or(names.as_slice(), |(columns, _)| columns);
            let mut reader = TypedQueryReader::open_with_metadata(part, columns, metadata)?;
            while let Some(batch) = reader.next_batch()? {
                let TypedColumn::NullableNumeric { values, nulls } = &batch[0] else {
                    return Err(ExecutionError::Invalid(
                        "nullable radix input changed type".into()
                    ));
                };
                let selected = typed_filtered.map(|(_, conditions)| {
                    (0..batch[0].len())
                        .map(|row| typed::passes(&batch, row, conditions))
                        .collect::<Vec<_>>()
                });
                count_column(values, Some(nulls), selected.as_deref(),
                    bits, shift, prefix_mask, &keys, histogram_sides, &mut histogram)?;
            }
        } else {
            let scan_columns = filtered.map_or(names.as_slice(), |(columns, _)| columns);
            let mut reader = NumericQueryReader::open_with_metadata(part, scan_columns, metadata)?;
            while let Some(batch) = reader.next_batch()? {
                let selected = filtered.map(|(_, conditions)| {
                    (0..batch[0].len())
                        .map(|row| batch_matches(&batch, row, conditions))
                        .collect::<Vec<_>>()
                });
                count_column(&batch[0], None, selected.as_deref(),
                    bits, shift, prefix_mask, &keys, histogram_sides, &mut histogram)?;
            }
        }
        if shared_prefix {
            histogram[1] = histogram[0];
        }
        for side in 0..2 {
            let mut remaining = ranks[side];
            let mut selected = false;
            for (bucket, &count) in histogram[side].iter().enumerate() {
                if remaining < count {
                    keys[side] |= (bucket as u128) << shift;
                    ranks[side] = remaining;
                    selected = true;
                    break;
                }
                remaining -= count;
            }
            if !selected {
                return Err(ExecutionError::Invalid(
                    "radix median row count differs from verified snapshot metadata".into()
                ));
            }
        }
    }
    if n % 2 == 1 {
        Ok(Some(render_key(keys[1], bits, signed)))
    } else {
        Ok(Some(render_middle_pair(keys[0], keys[1], bits, signed)))
    }
}

/// The non-nullable path uses the snapshot's exact verified row count.
pub(super) fn median(
    part: &Path,
    column: &str,
    metadata: &PartMetadata,
) -> Result<Option<String>, ExecutionError> {
    median_inner(part, column, metadata, metadata.rows, false, None, None)
}

/// The nullable path scans compact null maps once to derive the SQL count,
/// then excludes nulls from each bounded radix pass.
pub(super) fn median_nullable(
    part: &Path,
    column: &str,
    metadata: &PartMetadata,
) -> Result<Option<String>, ExecutionError> {
    let names = [column.to_owned()];
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, metadata)?;
    let mut count = 0u64;
    while let Some(nulls) = reader.next_null_batch()? {
        count = count.checked_add(nulls.iter().filter(|&&v| v == 0).count() as u64)
            .ok_or_else(|| ExecutionError::Invalid("median row count overflow".into()))?;
    }
    drop(reader);
    median_inner(part, column, metadata, count, true, None, None)
}

/// Filtered nullable medians combine typed predicates and NULL semantics
/// without retaining a large bitmap or all qualifying numeric values.
pub(super) fn median_nullable_filtered(
    part: &Path,
    column: &str,
    metadata: &PartMetadata,
    scan_columns: &[String],
    conditions: &[Vec<typed::Condition>],
) -> Result<Option<String>, ExecutionError> {
    let mut reader = TypedQueryReader::open_with_metadata(part, scan_columns, metadata)?;
    let mut selected_rows = 0u64;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !typed::is_null_at(&batch[0], row) && typed::passes(&batch, row, conditions) {
                selected_rows = selected_rows.checked_add(1)
                    .ok_or_else(|| ExecutionError::Invalid("median row count overflow".into()))?;
            }
        }
    }
    drop(reader);
    median_inner(part, column, metadata, selected_rows, true, None,
        Some((scan_columns, conditions)))
}

/// Exact filtered order statistics without storing rows or hash-set keys.
pub(super) fn median_filtered(
    part: &Path,
    column: &str,
    metadata: &PartMetadata,
    scan_columns: &[String],
    conditions: &[Vec<CompiledPredicate>],
) -> Result<Option<String>, ExecutionError> {
    let mut reader = NumericQueryReader::open_with_metadata(part, scan_columns, metadata)?;
    let mut selected_rows = 0_u64;
    while let Some(batch) = reader.next_batch()? {
        selected_rows = selected_rows
            .checked_add(count_batch_matches(&batch, conditions) as u64)
            .ok_or_else(|| ExecutionError::Invalid("median row count overflow".into()))?;
    }
    drop(reader);
    median_inner(
        part, column, metadata, selected_rows, false,
        Some((scan_columns, conditions)), None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_median_prefix_reuses_the_first_histogram() {
        let mut histogram = [[0u64; 256]; 2];
        let values = NumericColumn::UInt8(vec![1, 3, 3, 7]);
        count_column(&values, None, None, 8, 0, 0, &[0, 0], 1, &mut histogram)
            .unwrap();
        assert_eq!(histogram[0][1], 1);
        assert_eq!(histogram[0][3], 2);
        assert_eq!(histogram[0][7], 1);
        assert_eq!(histogram[1].iter().sum::<u64>(), 0);
        histogram[1] = histogram[0];
        assert_eq!(histogram[1], histogram[0]);
    }

    #[test]
    fn exact_middle_pair_handles_half_values_without_float_rounding() {
        let max = u128::MAX;
        assert_eq!(render_middle_pair(max - 1, max, 128, false), format!("{}.5", max - 1));
        assert_eq!(render_middle_pair(max, max, 128, false), max.to_string());
        assert_eq!(render_middle_pair(1, 2, 64, false), "1.5");

        let key = |value| sorted_key_signed(value, 128);
        assert_eq!(render_middle_pair(key(i128::MIN), key(i128::MAX), 128, true), "-0.5");
        assert_eq!(render_middle_pair(key(-3), key(-2), 128, true), "-2.5");
        assert_eq!(render_middle_pair(key(-1), key(0), 128, true), "-0.5");
        assert_eq!(render_middle_pair(key(9_007_199_254_740_993),
            key(9_007_199_254_740_994), 128, true), "9007199254740993.5");
    }

    #[test]
    fn signed_integer_key_order_matches_numeric_order() {
        let encoded = [-20_i128, -1, 0, 1, 100].map(|v| sorted_key_signed(v, 32));
        assert!(encoded.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(render_key(encoded[0], 32, true), "-20");
        assert_eq!(render_key(encoded[4], 32, true), "100");
    }
}
