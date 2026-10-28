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

    /// HLL registers form a mergeable max semilattice; partial scans may be
    /// reduced in any order without retaining individual distinct values.
    pub(super) fn merge(&mut self, other: &Self) {
        for (target, &source) in self.registers.iter_mut().zip(other.registers.iter()) {
            *target = (*target).max(source);
        }
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
    fn merge_is_idempotent_and_matches_a_single_global_scan() {
        let mut left = HyperLogLog::default();
        let mut right = HyperLogLog::default();
        let mut full = HyperLogLog::default();
        for value in 0..250_000u128 {
            let number = Number::Unsigned(value % 90_000);
            full.observe_number(number);
            if value % 2 == 0 {
                left.observe_number(number);
            } else {
                right.observe_number(number);
            }
        }
        left.merge(&right);
        assert_eq!(left.registers.as_slice(), full.registers.as_slice());
        let estimate = left.estimate();
        left.merge(&right);
        assert_eq!(left.estimate(), estimate);
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
