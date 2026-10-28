//! Fixed-memory cardinality sketch for high-cardinality analytical scans.
//! HLL with 2^14 registers has ~0.81% RSE, independent of input row count.
//! Not byte-for-byte compatible with ClickHouse's uniq() estimator.
use super::{ExecutionError, Number, number_at};
use rusthouse_storage::TypedColumn;

const PRECISION: u32 = 14;
const REGISTERS: usize = 1 << PRECISION;

pub(super) struct HyperLogLog {
    registers: Box<[u8; REGISTERS]>,
}

impl Default for HyperLogLog {
    fn default() -> Self {
        Self {
            registers: Box::new([0; REGISTERS]),
        }
    }
}

impl HyperLogLog {
    fn hash(value: &[u8]) -> u64 {
        // Deterministic across processes; avalanche distributes sequential IDs.
        let mut hash = 0xcbf29ce484222325u64;
        for &byte in value {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xff51afd7ed558ccd);
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xc4ceb9fe1a85ec53);
        hash ^ (hash >> 33)
    }

    fn add(&mut self, hash: u64) {
        let index = (hash >> (64 - PRECISION)) as usize;
        let rank = (hash << PRECISION).leading_zeros() as u8 + 1;
        self.registers[index] = self.registers[index].max(rank);
    }

    fn observe_number(&mut self, value: Number) {
        let hash = match value {
            Number::Unsigned(value) => Self::hash(&value.to_le_bytes()),
            Number::Signed(value) => Self::hash(&value.to_le_bytes()),
            Number::Float(value) => {
                let bits = if value == 0.0 {
                    0
                } else if value.is_nan() {
                    f64::NAN.to_bits()
                } else {
                    value.to_bits()
                };
                Self::hash(&bits.to_le_bytes())
            }
        };
        self.add(hash);
    }

    pub(super) fn observe(&mut self, column: &TypedColumn, row: usize) -> Result<(), ExecutionError> {
        match column {
            TypedColumn::Numeric(values) => self.observe_number(number_at(values, row)),
            TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 =>
                self.observe_number(number_at(values, row)),
            TypedColumn::Text(values) => {
                let value = values.get(row).ok_or_else(|| {
                    ExecutionError::Invalid("missing String value in uniq()".into())
                })?;
                self.add(Self::hash(value));
            }
            TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
                let value = values.get(row).ok_or_else(|| {
                    ExecutionError::Invalid("missing nullable String value in uniq()".into())
                })?;
                self.add(Self::hash(value));
            }
            TypedColumn::Uuid(values) => self.add(Self::hash(values[row].as_bytes())),
            TypedColumn::NullableNumeric { .. } | TypedColumn::NullableText { .. } => {}
        }
        Ok(())
    }

    /// Hash typed tuples without constructing or retaining row keys.
    /// Prefix every field with its type tag and byte length to separate
    /// ("ab","c") from ("a","bc") and distinguish NULL from non-NULL.
    pub(super) fn observe_tuple(
        &mut self,
        columns: &[TypedColumn],
        row: usize,
    ) -> Result<(), ExecutionError> {
        const OFFSET: u64 = 0xcbf29ce484222325;
        const PRIME: u64 = 0x100000001b3;
        fn feed(hash: &mut u64, bytes: &[u8]) {
            for &byte in bytes {
                *hash ^= u64::from(byte);
                *hash = (*hash).wrapping_mul(PRIME);
            }
        }
        let mut hash = OFFSET;
        for column in columns {
            let (tag, bytes): (u8, &[u8]) = match column {
                TypedColumn::Text(values) => (4, values.get(row).ok_or_else(|| {
                    ExecutionError::Invalid("missing tuple String value".into())
                })?),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 =>
                    (4, values.get(row).ok_or_else(|| {
                        ExecutionError::Invalid("missing tuple nullable String".into())
                    })?),
                TypedColumn::Uuid(values) => (5, values[row].as_bytes()),
                TypedColumn::Numeric(values) => {
                    match number_at(values, row) {
                        Number::Unsigned(value) => {
                            feed(&mut hash, &[1]);
                            feed(&mut hash, &16u64.to_le_bytes());
                            feed(&mut hash, &value.to_le_bytes());
                        }
                        Number::Signed(value) => {
                            feed(&mut hash, &[2]);
                            feed(&mut hash, &16u64.to_le_bytes());
                            feed(&mut hash, &value.to_le_bytes());
                        }
                        Number::Float(value) => {
                            let bits = if value == 0.0 { 0 } else if value.is_nan() {
                                f64::NAN.to_bits()
                            } else {
                                value.to_bits()
                            };
                            feed(&mut hash, &[3]);
                            feed(&mut hash, &8u64.to_le_bytes());
                            feed(&mut hash, &bits.to_le_bytes());
                        }
                    }
                    continue;
                }
                TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 => {
                    match number_at(values, row) {
                        Number::Unsigned(value) => {
                            feed(&mut hash, &[1]);
                            feed(&mut hash, &16u64.to_le_bytes());
                            feed(&mut hash, &value.to_le_bytes());
                        }
                        Number::Signed(value) => {
                            feed(&mut hash, &[2]);
                            feed(&mut hash, &16u64.to_le_bytes());
                            feed(&mut hash, &value.to_le_bytes());
                        }
                        Number::Float(value) => {
                            let bits = if value == 0.0 { 0 } else if value.is_nan() {
                                f64::NAN.to_bits()
                            } else { value.to_bits() };
                            feed(&mut hash, &[3]);
                            feed(&mut hash, &8u64.to_le_bytes());
                            feed(&mut hash, &bits.to_le_bytes());
                        }
                    }
                    continue;
                }
                TypedColumn::NullableText { .. } | TypedColumn::NullableNumeric { .. } => return Ok(()),
            };
            feed(&mut hash, &[tag]);
            feed(&mut hash, &(bytes.len() as u64).to_le_bytes());
            feed(&mut hash, bytes);
        }
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xff51afd7ed558ccd);
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(0xc4ceb9fe1a85ec53);
        self.add(hash ^ (hash >> 33));
        Ok(())
    }

    pub(super) fn estimate(&self) -> u64 {
        let m = REGISTERS as f64;
        let zero = self.registers.iter().filter(|&&rank| rank == 0).count();
        let harmonic = self.registers.iter()
            .map(|&rank| 2.0_f64.powi(-i32::from(rank)))
            .sum::<f64>();
        let alpha = 0.7213 / (1.0 + 1.079 / m);
        let raw = alpha * m * m / harmonic;
        let estimate = if raw <= 2.5 * m && zero > 0 {
            m * (m / zero as f64).ln()
        } else {
            raw
        };
        estimate.round().min(u64::MAX as f64) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_duplicates_and_small_counts_are_stable() {
        let mut sketch = HyperLogLog::default();
        assert_eq!(sketch.estimate(), 0);
        for _ in 0..100_000 {
            sketch.observe_number(Number::Unsigned(4));
        }
        assert_eq!(sketch.estimate(), 1);
        for value in 0..10_000 {
            sketch.observe_number(Number::Unsigned(value));
        }
        let estimate = sketch.estimate();
        assert!(estimate.abs_diff(10_000) < 400, "estimate={estimate}");
    }

    #[test]
    fn sketches_large_cardinality_without_retaining_keys() {
        let mut sketch = HyperLogLog::default();
        for value in 0..400_000 {
            sketch.observe_number(Number::Unsigned(value));
        }
        let estimate = sketch.estimate();
        assert!(estimate.abs_diff(400_000) < 16_000, "estimate={estimate}");
        assert_eq!(sketch.registers.len(), REGISTERS);
    }

    #[test]
    fn canonical_float_zero_and_nan() {
        let mut sketch = HyperLogLog::default();
        sketch.observe_number(Number::Float(-0.0));
        sketch.observe_number(Number::Float(0.0));
        assert_eq!(sketch.estimate(), 1);
        sketch.observe_number(Number::Float(f64::NAN));
        sketch.observe_number(Number::Float(-f64::NAN));
        assert_eq!(sketch.estimate(), 2);
    }
}


#[cfg(test)]
mod tuple_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn tuple_field_boundaries_are_not_ambiguous() {
        let first = TypedColumn::Text(
            rusthouse_storage::read_legacy_strings(&mut &b"\x02ab\x01a"[..], 2).unwrap(),
        );
        let second = TypedColumn::Text(
            rusthouse_storage::read_legacy_strings(&mut &b"\x01c\x02bc"[..], 2).unwrap(),
        );
        let mut sketch = HyperLogLog::default();
        let fields = [first, second];
        sketch.observe_tuple(&fields, 0).unwrap();
        sketch.observe_tuple(&fields, 1).unwrap();
        assert_eq!(sketch.estimate(), 2);
    }

    #[test]
    fn composite_numeric_tuple_is_bounded_and_deduplicates() {
        let x = TypedColumn::Numeric(NumericColumn::UInt64(vec![1, 2, 1]));
        let y = TypedColumn::Numeric(NumericColumn::UInt8(vec![1, 1, 1]));
        let mut sketch = HyperLogLog::default();
        let values = [x, y];
        for row in 0..3 {
            sketch.observe_tuple(&values, row).unwrap();
        }
        assert_eq!(sketch.estimate(), 2);
    }
}
