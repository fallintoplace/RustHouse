//! Bounded-memory exact median of physical integer columns.
//! One byte of the key is selected per immutable scan. Each pass uses only
//! 2 * 256 counters regardless of input cardinality.
use super::ExecutionError;
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
    if !signed {
        return key.to_string();
    }
    let sign_bit = 1_u128 << (bits - 1);
    let original = key ^ sign_bit;
    let extended = if original & sign_bit != 0 {
        original | !low_mask(bits)
    } else {
        original
    };
    (extended as i128).to_string()
}

fn key_as_f64(key: u128, bits: usize, signed: bool) -> f64 {
    if !signed {
        return key as f64;
    }
    let sign_bit = 1_u128 << (bits - 1);
    let original = key ^ sign_bit;
    let extended = if original & sign_bit != 0 {
        original | !low_mask(bits)
    } else {
        original
    };
    (extended as i128) as f64
}

/// Add the native integer batch directly to two radix histograms. Nullable
/// rows are skipped before key encoding, so NULLs never affect rank.
fn count_column(
    column: &NumericColumn,
    nulls: Option<&[u8]>,
    bits: usize,
    shift: usize,
    prefix_mask: u128,
    prefixes: &[u128; 2],
    histogram: &mut [[u64; 256]; 2],
) -> Result<(), ExecutionError> {
    let mut visit = |value: u128, row: usize| {
        if nulls.is_some_and(|map| map[row] != 0) {
            return;
        }
        for side in 0..2 {
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
        if nullable {
            let mut reader = TypedQueryReader::open_with_metadata(part, &names, metadata)?;
            while let Some(batch) = reader.next_batch()? {
                let TypedColumn::NullableNumeric { values, nulls } = &batch[0] else {
                    return Err(ExecutionError::Invalid(
                        "nullable radix input changed type".into()
                    ));
                };
                count_column(values, Some(nulls), bits, shift, prefix_mask, &keys, &mut histogram)?;
            }
        } else {
            let mut reader = NumericQueryReader::open_with_metadata(part, &names, metadata)?;
            while let Some(batch) = reader.next_batch()? {
                count_column(&batch[0], None, bits, shift, prefix_mask, &keys, &mut histogram)?;
            }
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
        let low = key_as_f64(keys[0], bits, signed);
        let high = key_as_f64(keys[1], bits, signed);
        Ok(Some((low / 2.0 + high / 2.0).to_string()))
    }
}

/// The non-nullable path uses the snapshot's exact verified row count.
pub(super) fn median(
    part: &Path,
    column: &str,
    metadata: &PartMetadata,
) -> Result<Option<String>, ExecutionError> {
    median_inner(part, column, metadata, metadata.rows, false)
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
    median_inner(part, column, metadata, count, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_integer_key_order_matches_numeric_order() {
        let encoded = [-20_i128, -1, 0, 1, 100].map(|v| sorted_key_signed(v, 32));
        assert!(encoded.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(render_key(encoded[0], 32, true), "-20");
        assert_eq!(render_key(encoded[4], 32, true), "100");
    }
}
