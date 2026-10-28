//! Exact O(domain) median for large 8/16-bit dimensions.
use super::{ExecutionError, Number};
use rusthouse_storage::NumericColumn;

pub(super) struct NarrowMedian {
    counts: Vec<u64>,
    rows: u64,
    width: u8,
    signed: bool,
}

impl NarrowMedian {
    pub(super) fn new(kind: &str) -> Option<Self> {
        let kind = kind.strip_prefix("Nullable(")
            .and_then(|inner| inner.strip_suffix(')')).unwrap_or(kind);
        let (width, signed) = match kind {
            "UInt8" => (8, false),
            "Int8" => (8, true),
            "UInt16" => (16, false),
            "Int16" => (16, true),
            _ => return None,
        };
        Some(Self {
            counts: vec![0; if width == 8 { 256 } else { 65_536 }],
            rows: 0,
            width,
            signed,
        })
    }

    pub(super) fn insert(&mut self, values: &NumericColumn, index: usize)
        -> Result<(), ExecutionError> {
        let key = match (self.width, self.signed, values) {
            (8, false, NumericColumn::UInt8(values)) => values[index] as u16,
            (8, true, NumericColumn::Int8(values)) => (values[index] as u8) as u16,
            (16, false, NumericColumn::UInt16(values)) => values[index],
            (16, true, NumericColumn::Int16(values)) => values[index] as u16,
            _ => return Err(ExecutionError::Invalid("narrow median column type changed".into())),
        };
        let count = &mut self.counts[key as usize];
        *count = count.checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("median frequency overflow".into()))?;
        self.rows = self.rows.checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("median row count overflow".into()))?;
        Ok(())
    }

    fn value(&self, key: usize) -> Number {
        if self.signed {
            if self.width == 8 {
                Number::Signed(i128::from(key as u8 as i8))
            } else {
                Number::Signed(i128::from(key as u16 as i16))
            }
        } else {
            Number::Unsigned(key as u128)
        }
    }

    pub(super) fn render(&self) -> String {
        if self.rows == 0 {
            return "NULL".to_owned();
        }
        let before = (self.rows - 1) / 2;
        let after = self.rows / 2;
        let domain = self.counts.len();
        let start = if self.signed { domain / 2 } else { 0 };
        let mut seen = 0u64;
        let mut left = None;
        let mut right = None;
        // Signed order starts at the high-bit half, then wraps to zero.
        for rank in 0..domain {
            let key = (start + rank) % domain;
            let count = self.counts[key];
            if count == 0 {
                continue;
            }
            let end = seen + count;
            if left.is_none() && before < end {
                left = Some(self.value(key));
            }
            if after < end {
                right = Some(self.value(key));
                break;
            }
            seen = end;
        }
        let (Some(a), Some(b)) = (left, right) else {
            unreachable!("nonempty histogram must contain its central positions")
        };
        if self.rows % 2 == 1 {
            return b.to_string();
        }
        let float = |value: Number| match value {
            Number::Unsigned(x) => x as f64,
            Number::Signed(x) => x as f64,
            Number::Float(x) => x,
        };
        (float(a) / 2.0 + float(b) / 2.0).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_median_supports_more_than_a_hundred_thousand_rows() {
        let mut median = NarrowMedian::new("UInt8").unwrap();
        let input = NumericColumn::UInt8(vec![3, 1, 2, 2]);
        for _ in 0..100_000 {
            for i in 0..4 {
                median.insert(&input, i).unwrap();
            }
        }
        assert_eq!(median.render(), "2");
    }

    #[test]
    fn signed_order_and_interpolation_match_generic_median() {
        let mut median = NarrowMedian::new("Nullable(Int16)").unwrap();
        let values = NumericColumn::Int16(vec![-30, 10, -20, 20]);
        for i in 0..4 {
            median.insert(&values, i).unwrap();
        }
        assert_eq!(median.render(), "-5");
        assert_eq!(NarrowMedian::new("Int8").unwrap().render(), "NULL");
        assert!(NarrowMedian::new("UInt32").is_none());
    }
}
