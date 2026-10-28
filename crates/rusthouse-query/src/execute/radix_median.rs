//! Bounded-memory exact median of physical integer columns.
//! One byte of the key is selected per immutable scan. Each pass uses only
//! 2 * 256 counters regardless of input cardinality.
use super::ExecutionError;
use rusthouse_storage::{NumericColumn, NumericQueryReader, PartMetadata};
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

/// Exact integer selection, with Float64 interpolation for even row counts.
/// No data rows are retained between the passes.
pub(super) fn median(
    part: &Path,
    column: &str,
    metadata: &PartMetadata,
) -> Result<Option<String>, ExecutionError> {
    let kind = metadata.columns.iter().find(|item| item.name == column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown column: {column}")))?;
    let (bytes, signed) = width_and_sign(&kind.data_type)
        .ok_or_else(|| ExecutionError::Invalid("radix median requires integer values".into()))?;
    let n = metadata.rows;
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
        let mut reader = NumericQueryReader::open_with_metadata(part, &names, metadata)?;
        while let Some(batch) = reader.next_batch()? {
            let mut visit = |value: u128| {
                for side in 0..2 {
                    if value & prefix_mask != keys[side] & prefix_mask {
                        continue;
                    }
                    let bucket = ((value >> shift) & 255) as usize;
                    histogram[side][bucket] += 1;
                }
            };
            macro_rules! unsigned {
                ($values:expr) => {
                    for &value in $values {
                        visit(sorted_key_unsigned(u128::from(value)));
                    }
                };
            }
            macro_rules! signed {
                ($values:expr) => {
                    for &value in $values {
                        visit(sorted_key_signed(i128::from(value), bits));
                    }
                };
            }
            match &batch[0] {
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
                    "radix median column changed type during scan".into()
                )),
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
