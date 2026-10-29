//! Fixed-memory cardinality sketch for high-cardinality analytical scans.
//! HLL with 2^14 registers has ~0.81% RSE, independent of input row count.
//! Not byte-for-byte compatible with ClickHouse's uniq() estimator.
use super::{ExecutionError, Number, number_at};
use rusthouse_storage::TypedColumn;

const PRECISION: u32 = 14;
const REGISTERS: usize = 1 << PRECISION;
const MAX_REGISTER_RANK: u8 = (64 - PRECISION + 1) as u8;

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
        // A zero suffix has rank 64 - PRECISION + 1, not 65.
        // Keep every emitted register valid for strict shard-state decoding.
        let rank = ((hash << PRECISION).leading_zeros() + 1)
            .min(u32::from(MAX_REGISTER_RANK)) as u8;
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

    /// A portable, bounded shard state. Each register is two lowercase hex
    /// characters, independent of byte order, architecture, and input rows.
    pub(super) fn encode_state(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut bytes = Vec::with_capacity(REGISTERS * 2);
        for &register in self.registers.iter() {
            bytes.push(HEX[(register >> 4) as usize]);
            bytes.push(HEX[(register & 15) as usize]);
        }
        String::from_utf8(bytes).expect("ASCII hex bytes are valid UTF-8")
    }

    fn decode_state(value: &str) -> Result<Self, String> {
        let bytes = value.as_bytes();
        if bytes.len() != REGISTERS * 2 {
            return Err("invalid HLL register count".into());
        }
        let digit = |value: u8| match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            _ => None,
        };
        let mut state = Self::default();
        for (index, bytes) in bytes.chunks_exact(2).enumerate() {
            let high = digit(bytes[0]).ok_or("invalid HLL hex state")?;
            let low = digit(bytes[1]).ok_or("invalid HLL hex state")?;
            let register = (high << 4) | low;
            if register > MAX_REGISTER_RANK {
                return Err("invalid HLL register rank".into());
            }
            state.registers[index] = register;
        }
        Ok(state)
    }

    pub(super) fn merge(&mut self, other: &Self) {
        for (target, &register) in self.registers.iter_mut().zip(other.registers.iter()) {
            *target = (*target).max(register);
        }
    }

    pub(super) fn estimate(&self) -> u64 {
        let register_count = REGISTERS as f64;
        let (zero_register_count, harmonic) = self.registers.iter().fold(
            (0usize, 0.0_f64),
            |(zeros, sum), &rank| {
                (zeros + usize::from(rank == 0), sum + 2.0_f64.powi(-i32::from(rank)))
            },
        );
        let alpha = 0.7213 / (1.0 + 1.079 / register_count);
        let raw_estimate = alpha * register_count * register_count / harmonic;
        let estimate = if raw_estimate <= 2.5 * register_count && zero_register_count > 0 {
            register_count * (register_count / zero_register_count as f64).ln()
        } else {
            raw_estimate
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
    fn all_zero_hash_stays_in_portable_register_range() {
        let mut sketch = HyperLogLog::default();
        sketch.add(0);
        assert_eq!(sketch.registers[0], (64 - PRECISION + 1) as u8);
        let encoded = sketch.encode_state();
        assert!(HyperLogLog::decode_state(&encoded).is_ok());
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


/// Decode strict, fixed-size shard states and calculate the global union.
pub(super) fn merge_state_hex_values(states: &[&str]) -> Result<u64, String> {
    if states.is_empty() {
        return Err("cannot merge zero HLL states".into());
    }
    let mut union = HyperLogLog::default();
    for &state in states {
        union.merge(&HyperLogLog::decode_state(state)?);
    }
    Ok(union.estimate())
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn serialized_partial_states_have_exact_global_union() {
        let mut first = HyperLogLog::default();
        let mut second = HyperLogLog::default();
        let mut combined = HyperLogLog::default();
        for index in 0..200_000 {
            let value = Number::Unsigned(index % 125_000);
            combined.observe_number(value);
            if index % 2 == 0 {
                first.observe_number(value);
            } else {
                second.observe_number(value);
            }
        }
        let first = first.encode_state();
        let second = second.encode_state();
        assert_eq!(
            merge_state_hex_values(&[&first, &second]).unwrap(),
            combined.estimate()
        );
        let empty = HyperLogLog::default().encode_state();
        assert_eq!(merge_state_hex_values(&[&empty]).unwrap(), 0);
    }

    #[test]
    fn malformed_truncated_or_noncanonical_hll_states_are_rejected() {
        let valid = HyperLogLog::default().encode_state();
        assert!(merge_state_hex_values(&[]).is_err());
        assert!(merge_state_hex_values(&[&valid[..valid.len()-2]]).is_err());
        assert!(merge_state_hex_values(&[&format!("ff{}", &valid[2..])]).is_err());
        assert!(merge_state_hex_values(&[&format!("zz{}", &valid[2..])]).is_err());
        assert!(merge_state_hex_values(&[&format!("AA{}", &valid[2..])]).is_err());
    }
}
