mod comparison;

use self::comparison::*;

use super::{ExecutionError, Number, NumberKey, number_at};
use crate::{Operator, Predicate, Select};
use rusthouse_storage::NumericColumn;
use std::collections::HashSet;

// Short IN lists are cheap to scan; long lists benefit from constant-time lookups.
const HASHED_IN_MIN_VALUES: usize = 16;

pub(super) enum CompiledTest {
    Comparison {
        operator: Operator,
        target: Number,
    },
    In(Vec<Number>),
    NotIn(Vec<Number>),
    NotBetween {
        lower: Number,
        upper: Number,
    },
    DistinctFrom {
        target: Number,
        negated: bool,
    },
    /// Constant predicate for non-Nullable generated numeric columns.
    Always(bool),
    InSet {
        targets: HashSet<NumberKey>,
        negated: bool,
    },
}

pub(super) fn compile_in_test(targets: Vec<Number>, negated: bool) -> CompiledTest {
    if targets.len() < HASHED_IN_MIN_VALUES {
        if negated {
            CompiledTest::NotIn(targets)
        } else {
            CompiledTest::In(targets)
        }
    } else {
        CompiledTest::InSet {
            targets: targets.into_iter().map(NumberKey::from).collect(),
            negated,
        }
    }
}

impl CompiledTest {
    pub(super) fn matches(&self, value: Number) -> bool {
        match self {
            Self::Comparison { operator, target } => number_matches(value, *target, *operator),
            Self::DistinctFrom { target, negated } => {
                let equal = matches!((value, *target), (Number::Float(left), Number::Float(right)) if left.is_nan() && right.is_nan())
                    || number_matches(value, *target, Operator::Eq);
                equal == *negated
            }
            Self::Always(value) => *value,
            Self::In(targets) => targets
                .iter()
                .any(|target| number_matches(value, *target, Operator::Eq)),
            Self::NotIn(targets) => targets
                .iter()
                .all(|target| !number_matches(value, *target, Operator::Eq)),
            Self::NotBetween { lower, upper } => {
                !number_matches(value, *lower, Operator::Ge)
                    || !number_matches(value, *upper, Operator::Le)
            }
            Self::InSet { targets, negated } => {
                // Float NaN does not compare equal to any IN-list value.
                let contains = !matches!(value, Number::Float(number) if number.is_nan())
                    && targets.contains(&NumberKey::from(value));
                contains != *negated
            }
        }
    }
}

pub(super) struct CompiledPredicate {
    index: usize,
    test: CompiledTest,
}

pub(super) fn batch_matches(
    batch: &[NumericColumn],
    row: usize,
    branches: &[Vec<CompiledPredicate>],
) -> bool {
    branches.iter().any(|predicates| {
        predicates.iter().all(|predicate| {
            predicate
                .test
                .matches(number_at(&batch[predicate.index], row))
        })
    })
}

/// Count a numeric batch without constructing a Number for every row when
/// a single simple comparison can use the underlying typed column directly.
pub(super) fn count_batch_matches(
    batch: &[NumericColumn],
    branches: &[Vec<CompiledPredicate>],
) -> usize {
    if let [predicates] = branches {
        if predicates.is_empty() {
            return batch.first().map_or(0, NumericColumn::len);
        }
        if let [predicate] = predicates.as_slice() {
            if let CompiledTest::Comparison { operator, target } = &predicate.test {
                if !matches!(operator, Operator::Modulo(_) | Operator::ModuloNe(_)) {
                    macro_rules! count_direct {
                        ($values:expr, $target:expr, $operator:expr, $wide:ty) => {
                            $values
                                .iter()
                                .filter(|&&value| compare(<$wide>::from(value), $target, $operator))
                                .count()
                        };
                    }
                    match (&batch[predicate.index], *target) {
                        (NumericColumn::UInt8(values), Number::Unsigned(target)) => {
                            return count_direct!(values, target, *operator, u128);
                        }
                        (NumericColumn::UInt16(values), Number::Unsigned(target)) => {
                            return count_direct!(values, target, *operator, u128);
                        }
                        (NumericColumn::UInt32(values), Number::Unsigned(target)) => {
                            return count_direct!(values, target, *operator, u128);
                        }
                        (NumericColumn::UInt64(values), Number::Unsigned(target)) => {
                            return count_direct!(values, target, *operator, u128);
                        }
                        (NumericColumn::UInt128(values), Number::Unsigned(target)) => {
                            return count_direct!(values, target, *operator, u128);
                        }
                        (NumericColumn::Int8(values), Number::Signed(target)) => {
                            return count_direct!(values, target, *operator, i128);
                        }
                        (NumericColumn::Int16(values), Number::Signed(target)) => {
                            return count_direct!(values, target, *operator, i128);
                        }
                        (NumericColumn::Int32(values), Number::Signed(target)) => {
                            return count_direct!(values, target, *operator, i128);
                        }
                        (NumericColumn::Int64(values), Number::Signed(target)) => {
                            return count_direct!(values, target, *operator, i128);
                        }
                        (NumericColumn::Int128(values), Number::Signed(target)) => {
                            return count_direct!(values, target, *operator, i128);
                        }
                        (NumericColumn::Float32(values), Number::Float(target)) => {
                            return count_direct!(values, target, *operator, f64);
                        }
                        (NumericColumn::Float64(values), Number::Float(target)) => {
                            return count_direct!(values, target, *operator, f64);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    // Compound OR/AND, IN and modulo retain the original semantics.
    (0..batch.first().map_or(0, NumericColumn::len))
        .filter(|&row| batch_matches(batch, row, branches))
        .count()
}

pub(super) fn compile_part_where(
    query: &Select,
    scan_columns: &mut Vec<String>,
    mut convert: impl FnMut(&str, &str) -> Result<Number, ExecutionError>,
) -> Result<Vec<Vec<CompiledPredicate>>, ExecutionError> {
    std::iter::once(query.predicates.as_slice())
        .chain(query.or_predicates.iter().map(Vec::as_slice))
        .map(|predicates| compile_part_predicates(predicates, scan_columns, &mut convert))
        .collect()
}

pub(super) fn compile_numbers_predicates(
    query: &Select,
) -> Result<Vec<Vec<CompiledPredicate>>, ExecutionError> {
    let mut scan_columns = vec!["number".into()];
    compile_part_where(query, &mut scan_columns, |column, value| {
        if column != "number" {
            return Err(ExecutionError::Invalid(
                "numbers(N) exposes only the number column".into(),
            ));
        }
        parse_numeric("UInt64", value)
    })
}

/// Count a single AND branch over an increasing generated UInt64 sequence
/// without visiting its rows. Unsupported OR, IN and modulo predicates keep
/// the normal row-wise executor. Arithmetic uses u128 at UInt64 boundaries.
pub(super) fn count_monotonic_numbers(
    branches: &[Vec<CompiledPredicate>],
    start: u64,
    rows: u64,
    step: u64,
) -> Option<u64> {
    let [conjunction] = branches else {
        return None;
    };
    if step == 0 {
        return None;
    }
    if let [predicate] = conjunction {
        if let CompiledTest::Always(value) = &predicate.test {
            return Some(if *value { rows } else { 0 });
        }
    }

    let start = u128::from(start);
    let step = u128::from(step);
    let mut lower = 0u128;
    let mut upper = u128::from(rows);
    let mut excluded = Vec::new();

    for predicate in conjunction {
        let CompiledTest::Comparison {
            operator,
            target: Number::Unsigned(target),
        } = &predicate.test
        else {
            return None;
        };
        let index_of = || {
            if *target < start || (*target - start) % step != 0 {
                None
            } else {
                Some((*target - start) / step)
            }
        };
        match operator {
            Operator::Eq => {
                if let Some(index) = index_of() {
                    lower = lower.max(index);
                    upper = upper.min(index.saturating_add(1));
                } else {
                    upper = 0;
                }
            }
            Operator::Ne => {
                if let Some(index) = index_of() {
                    if !excluded.contains(&index) {
                        excluded.push(index);
                    }
                }
            }
            Operator::Lt => {
                let end = if *target <= start {
                    0
                } else {
                    (*target - start).div_ceil(step)
                };
                upper = upper.min(end);
            }
            Operator::Le => {
                let end = if *target < start {
                    0
                } else {
                    ((*target - start) / step).saturating_add(1)
                };
                upper = upper.min(end);
            }
            Operator::Gt => {
                let begin = if *target < start {
                    0
                } else {
                    ((*target - start) / step).saturating_add(1)
                };
                lower = lower.max(begin);
            }
            Operator::Ge => {
                let begin = if *target <= start {
                    0
                } else {
                    (*target - start).div_ceil(step)
                };
                lower = lower.max(begin);
            }
            Operator::Modulo(_) | Operator::ModuloNe(_) => return None,
        }
    }

    let remaining = upper.saturating_sub(lower);
    let excluded_in_range = excluded
        .iter()
        .filter(|&&index| index >= lower && index < upper)
        .count() as u128;
    u64::try_from(remaining - excluded_in_range).ok()
}

pub(super) fn number_matches_all(value: Number, branches: &[Vec<CompiledPredicate>]) -> bool {
    branches.iter().any(|predicates| {
        predicates
            .iter()
            .all(|predicate| predicate.test.matches(value))
    })
}

pub(super) fn parse_numeric(data_type: &str, text: &str) -> Result<Number, ExecutionError> {
    let value = match data_type {
        "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" => {
            text.parse::<u128>().ok().map(Number::Unsigned)
        }
        "Int8" | "Int16" | "Int32" | "Int64" | "Int128" => {
            text.parse::<i128>().ok().map(Number::Signed)
        }
        "Float32" => text
            .parse::<f32>()
            .ok()
            .map(|value| Number::Float(f64::from(value))),
        "Float64" => text.parse::<f64>().ok().map(Number::Float),
        other => {
            return Err(ExecutionError::Invalid(format!(
                "unsupported WHERE column type: {other}"
            )));
        }
    };
    value.ok_or_else(|| ExecutionError::Invalid(format!("invalid numeric WHERE value: {text}")))
}

pub(super) fn number_matches(value: Number, target: Number, operator: Operator) -> bool {
    if let Operator::Modulo(modulus) | Operator::ModuloNe(modulus) = operator {
        let equal = match (value, target) {
            (Number::Unsigned(value), Number::Unsigned(target)) => {
                value % u128::from(modulus) == target
            }
            (Number::Signed(value), Number::Signed(target)) => {
                value % i128::from(modulus) == target
            }
            _ => return false,
        };
        return if matches!(operator, Operator::ModuloNe(_)) {
            !equal
        } else {
            equal
        };
    }
    match (value, target) {
        (Number::Unsigned(value), Number::Unsigned(target)) => compare(value, target, operator),
        (Number::Signed(value), Number::Signed(target)) => compare(value, target, operator),
        (Number::Float(value), Number::Float(target)) => compare(value, target, operator),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_or_branches_using_the_same_numeric_conversion() {
        let query =
            crate::parse_select("SELECT number FROM numbers(10) WHERE number = 1 OR number = 3")
                .unwrap();
        let branches = compile_numbers_predicates(&query).unwrap();
        assert_eq!(branches.len(), 2);
        assert!(number_matches_all(Number::Unsigned(1), &branches));
        assert!(number_matches_all(Number::Unsigned(3), &branches));
        assert!(!number_matches_all(Number::Unsigned(2), &branches));
    }

    #[test]
    fn hashed_in_and_not_in_keep_membership_semantics() {
        let values = (0..16).map(Number::Unsigned).collect::<Vec<_>>();
        let included = compile_in_test(values.clone(), false);
        let excluded = compile_in_test(values, true);
        assert!(matches!(&included, CompiledTest::InSet { .. }));
        assert!(matches!(&excluded, CompiledTest::InSet { .. }));
        assert!(included.matches(Number::Unsigned(0)));
        assert!(included.matches(Number::Unsigned(15)));
        assert!(!included.matches(Number::Unsigned(16)));
        assert!(!excluded.matches(Number::Unsigned(0)));
        assert!(excluded.matches(Number::Unsigned(16)));
    }

    #[test]
    fn batched_counts_match_rowwise_predicates_across_numeric_types() {
        for (batch, target) in [
            (NumericColumn::UInt8(vec![0, 1, 3, 5]), Number::Unsigned(3)),
            (
                NumericColumn::UInt64(vec![0, 1, 3, u64::MAX]),
                Number::Unsigned(3),
            ),
            (NumericColumn::Int64(vec![-9, -1, 3, 7]), Number::Signed(3)),
            (
                NumericColumn::Float64(vec![f64::NAN, -1.0, 3.0, 5.0]),
                Number::Float(3.0),
            ),
        ] {
            for operator in [
                Operator::Eq,
                Operator::Ne,
                Operator::Lt,
                Operator::Le,
                Operator::Gt,
                Operator::Ge,
            ] {
                let branches = vec![vec![CompiledPredicate {
                    index: 0,
                    test: CompiledTest::Comparison { operator, target },
                }]];
                let expected = (0..batch.len())
                    .filter(|&row| batch_matches(std::slice::from_ref(&batch), row, &branches))
                    .count();
                assert_eq!(
                    count_batch_matches(std::slice::from_ref(&batch), &branches),
                    expected
                );
            }
        }
    }

    #[test]
    fn batched_counts_preserve_or_and_modulo_fallback() {
        let batch = [NumericColumn::UInt64(vec![0, 1, 2, 3, 4, 5])];
        let make = |operator, target| CompiledPredicate {
            index: 0,
            test: CompiledTest::Comparison {
                operator,
                target: Number::Unsigned(target),
            },
        };
        for branches in [
            vec![vec![make(Operator::Modulo(2), 1)]],
            vec![vec![make(Operator::Lt, 2)], vec![make(Operator::Ge, 4)]],
            vec![vec![make(Operator::Gt, 1), make(Operator::Lt, 5)]],
        ] {
            let expected = (0..batch[0].len())
                .filter(|&row| batch_matches(&batch, row, &branches))
                .count();
            assert_eq!(count_batch_matches(&batch, &branches), expected);
        }
        assert_eq!(count_batch_matches(&batch, &[vec![]]), 6);
    }

    #[test]
    fn hashed_float_in_normalizes_signed_zero_but_not_nan() {
        let values = (1..16)
            .map(|value| Number::Float(f64::from(value)))
            .chain(std::iter::once(Number::Float(-0.0)))
            .collect();
        let included = compile_in_test(values, false);
        assert!(included.matches(Number::Float(0.0)));
        assert!(!included.matches(Number::Float(f64::NAN)));
    }
}

#[cfg(test)]
mod generated_constant_count_tests {
    use super::*;

    #[test]
    fn constant_predicates_do_not_iterate_billions_of_rows() {
        for (value, expected) in [(true, 1_000_000_000), (false, 0)] {
            assert_eq!(
                count_monotonic_numbers(&[vec![CompiledPredicate {
                    index: 0,
                    test: CompiledTest::Always(value),
                }]], 0, 1_000_000_000, 1),
                Some(expected),
            );
        }
    }
}
