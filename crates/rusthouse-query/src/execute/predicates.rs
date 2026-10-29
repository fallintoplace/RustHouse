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
                if matches!(value, Number::Float(n) if n.is_nan())
                    || matches!(*lower, Number::Float(n) if n.is_nan())
                    || matches!(*upper, Number::Float(n) if n.is_nan())
                {
                    return false;
                }
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

/** Evaluate a predicate tree that only depends on non-nullable column
 * properties. A known-true OR branch dominates unknown branches, while
 * false branches can be ignored. Unknown predicates require the normal scan. */
pub(super) fn constant_predicate_result(branches: &[Vec<CompiledPredicate>]) -> Option<bool> {
    let mut unknown_branch = false;
    for branch in branches {
        let mut unknown = false;
        let mut impossible = false;
        for predicate in branch {
            match predicate.test {
                CompiledTest::Always(false) => {
                    impossible = true;
                    break;
                }
                CompiledTest::Always(true) => {}
                _ => unknown = true,
            }
        }
        if !impossible && !unknown {
            return Some(true);
        }
        if !impossible && unknown {
            unknown_branch = true;
        }
    }
    if unknown_branch { None } else { Some(false) }
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
            if let CompiledTest::InSet { targets, negated } = &predicate.test {
                macro_rules! count_hashed_in {
                    ($values:expr, $variant:ident, $wide:ty) => {{
                        return $values
                            .iter()
                            .filter(|&&value| {
                                let key = NumberKey::$variant(<$wide>::from(value));
                                targets.contains(&key) != *negated
                            })
                            .count();
                    }};
                }
                // Eight-bit domains contain only 256 values. Resolve the
                // hashed membership once per possible byte, then count rows
                // with a cache-friendly array lookup instead of hashing
                // each input element.
                match &batch[predicate.index] {
                    NumericColumn::UInt8(values) => {
                        if values.len() < 512 {
                            return values
                                .iter()
                                .filter(|&&value| {
                                    targets.contains(&NumberKey::Unsigned(u128::from(value)))
                                        != *negated
                                })
                                .count();
                        }
                        let mut matches = [false; 256];
                        for (byte, result) in matches.iter_mut().enumerate() {
                            *result = targets.contains(&NumberKey::Unsigned(byte as u128));
                        }
                        return values
                            .iter()
                            .filter(|&&value| matches[usize::from(value)] != *negated)
                            .count();
                    }
                    NumericColumn::Int8(values) => {
                        if values.len() < 512 {
                            return values
                                .iter()
                                .filter(|&&value| {
                                    targets.contains(&NumberKey::Signed(i128::from(value)))
                                        != *negated
                                })
                                .count();
                        }
                        let mut matches = [false; 256];
                        for (byte, result) in matches.iter_mut().enumerate() {
                            *result = targets.contains(&NumberKey::Signed(i128::from(byte as i8)));
                        }
                        return values
                            .iter()
                            .filter(|&&value| matches[value as u8 as usize] != *negated)
                            .count();
                    }
                    NumericColumn::UInt16(values) => {
                        if values.len() >= 2048 {
                            let mut bitmap = [0u64; 1024];
                            for target in targets {
                                if let NumberKey::Unsigned(value) = target {
                                    if let Ok(slot) = u16::try_from(*value) {
                                        let index = usize::from(slot);
                                        bitmap[index / 64] |= 1u64 << (index % 64);
                                    }
                                }
                            }
                            return values.iter().filter(|&&value| {
                                let index = usize::from(value);
                                ((bitmap[index / 64] >> (index % 64)) & 1 != 0) != *negated
                            }).count();
                        }
                        count_hashed_in!(values, Unsigned, u128)
                    },
                    NumericColumn::UInt32(v) => count_hashed_in!(v, Unsigned, u128),
                    NumericColumn::UInt64(v) => count_hashed_in!(v, Unsigned, u128),
                    NumericColumn::UInt128(v) => count_hashed_in!(v, Unsigned, u128),

                    NumericColumn::Int16(v) => count_hashed_in!(v, Signed, i128),
                    NumericColumn::Int32(v) => count_hashed_in!(v, Signed, i128),
                    NumericColumn::Int64(v) => count_hashed_in!(v, Signed, i128),
                    NumericColumn::Int128(v) => count_hashed_in!(v, Signed, i128),
                    NumericColumn::Float32(_) | NumericColumn::Float64(_) => {}
                }
            }
            if let CompiledTest::NotBetween { lower, upper } = &predicate.test {
                macro_rules! count_outside {
                    ($values:expr, $lower:expr, $upper:expr, $wide:ty) => {{
                        return $values
                            .iter()
                            .filter(|&&value| {
                                let value = <$wide>::from(value);
                                value.partial_cmp(&$lower).is_some()
                                    && value.partial_cmp(&$upper).is_some()
                                    && !(value >= $lower && value <= $upper)
                            })
                            .count();
                    }};
                }
                match (&batch[predicate.index], *lower, *upper) {
                    (NumericColumn::UInt8(v), Number::Unsigned(a), Number::Unsigned(b)) => {
                        count_outside!(v, a, b, u128)
                    }
                    (NumericColumn::UInt16(v), Number::Unsigned(a), Number::Unsigned(b)) => {
                        count_outside!(v, a, b, u128)
                    }
                    (NumericColumn::UInt32(v), Number::Unsigned(a), Number::Unsigned(b)) => {
                        count_outside!(v, a, b, u128)
                    }
                    (NumericColumn::UInt64(v), Number::Unsigned(a), Number::Unsigned(b)) => {
                        count_outside!(v, a, b, u128)
                    }
                    (NumericColumn::UInt128(v), Number::Unsigned(a), Number::Unsigned(b)) => {
                        count_outside!(v, a, b, u128)
                    }
                    (NumericColumn::Int8(v), Number::Signed(a), Number::Signed(b)) => {
                        count_outside!(v, a, b, i128)
                    }
                    (NumericColumn::Int16(v), Number::Signed(a), Number::Signed(b)) => {
                        count_outside!(v, a, b, i128)
                    }
                    (NumericColumn::Int32(v), Number::Signed(a), Number::Signed(b)) => {
                        count_outside!(v, a, b, i128)
                    }
                    (NumericColumn::Int64(v), Number::Signed(a), Number::Signed(b)) => {
                        count_outside!(v, a, b, i128)
                    }
                    (NumericColumn::Int128(v), Number::Signed(a), Number::Signed(b)) => {
                        count_outside!(v, a, b, i128)
                    }
                    (NumericColumn::Float32(v), Number::Float(a), Number::Float(b)) => {
                        count_outside!(v, a, b, f64)
                    }
                    (NumericColumn::Float64(v), Number::Float(a), Number::Float(b)) => {
                        count_outside!(v, a, b, f64)
                    }
                    _ => {}
                }
            }
            if let CompiledTest::Always(value) = &predicate.test {
                return if *value {
                    batch.first().map_or(0, NumericColumn::len)
                } else {
                    0
                };
            }
            if let CompiledTest::Comparison { operator, target } = &predicate.test {
                if let Operator::Modulo(modulus) | Operator::ModuloNe(modulus) = *operator {
                    let negated = matches!(operator, Operator::ModuloNe(_));
                    if modulus != 0 {
                        macro_rules! modulo_count {
                            ($values:expr, $target:expr, $wide:ty) => {{
                                let divisor = <$wide>::from(modulus);
                                return $values
                                    .iter()
                                    .filter(|&&value| {
                                        ((<$wide>::from(value) % divisor) == $target) != negated
                                    })
                                    .count();
                            }};
                        }
                        match (&batch[predicate.index], *target) {
                            (NumericColumn::UInt8(v), Number::Unsigned(t)) => {
                                modulo_count!(v, t, u128)
                            }
                            (NumericColumn::UInt16(v), Number::Unsigned(t)) => {
                                modulo_count!(v, t, u128)
                            }
                            (NumericColumn::UInt32(v), Number::Unsigned(t)) => {
                                modulo_count!(v, t, u128)
                            }
                            (NumericColumn::UInt64(v), Number::Unsigned(t)) => {
                                modulo_count!(v, t, u128)
                            }
                            (NumericColumn::UInt128(v), Number::Unsigned(t)) => {
                                modulo_count!(v, t, u128)
                            }
                            (NumericColumn::Int8(v), Number::Signed(t)) => {
                                modulo_count!(v, t, i128)
                            }
                            (NumericColumn::Int16(v), Number::Signed(t)) => {
                                modulo_count!(v, t, i128)
                            }
                            (NumericColumn::Int32(v), Number::Signed(t)) => {
                                modulo_count!(v, t, i128)
                            }
                            (NumericColumn::Int64(v), Number::Signed(t)) => {
                                modulo_count!(v, t, i128)
                            }
                            (NumericColumn::Int128(v), Number::Signed(t)) => {
                                modulo_count!(v, t, i128)
                            }
                            _ => {}
                        }
                    }
                }
                if !matches!(operator, Operator::Modulo(_) | Operator::ModuloNe(_)) {
                    macro_rules! count_direct {
                        ($values:expr, $target:expr, $operator:expr, $wide:ty) => {{
                            let target: $wide = $target;
                            match $operator {
                                Operator::Eq => $values
                                    .iter()
                                    .filter(|&&value| <$wide>::from(value) == target)
                                    .count(),
                                Operator::Ne => $values
                                    .iter()
                                    .filter(|&&value| <$wide>::from(value) != target)
                                    .count(),
                                Operator::Lt => $values
                                    .iter()
                                    .filter(|&&value| <$wide>::from(value) < target)
                                    .count(),
                                Operator::Le => $values
                                    .iter()
                                    .filter(|&&value| <$wide>::from(value) <= target)
                                    .count(),
                                Operator::Gt => $values
                                    .iter()
                                    .filter(|&&value| <$wide>::from(value) > target)
                                    .count(),
                                Operator::Ge => $values
                                    .iter()
                                    .filter(|&&value| <$wide>::from(value) >= target)
                                    .count(),
                                Operator::Modulo(_) | Operator::ModuloNe(_) => {
                                    unreachable!("modulo is handled separately")
                                }
                            }
                        }};
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
    // WHERE key >= lower AND key < upper is a common billion-row COUNT.
    // On one numeric column, evaluate both comparisons on the native values
    // instead of boxing each element and dispatching the predicate tree twice.
    if let [predicates] = branches {
        if let [first, second] = predicates.as_slice() {
            if first.index == second.index {
                if let (
                    CompiledTest::Comparison {
                        operator: first_op,
                        target: first_target,
                    },
                    CompiledTest::Comparison {
                        operator: second_op,
                        target: second_target,
                    },
                ) = (&first.test, &second.test)
                {
                    if !matches!(first_op, Operator::Modulo(_) | Operator::ModuloNe(_))
                        && !matches!(second_op, Operator::Modulo(_) | Operator::ModuloNe(_))
                    {
                        macro_rules! count_pair {
                            ($values:expr, $a:expr, $b:expr, $first:expr, $second:expr, $wide:ty) => {
                                $values
                                    .iter()
                                    .filter(|&&value| {
                                        let value = <$wide>::from(value);
                                        compare(value, $a, $first) && compare(value, $b, $second)
                                    })
                                    .count()
                            };
                        }
                        match (&batch[first.index], *first_target, *second_target) {
                            (
                                NumericColumn::UInt8(values),
                                Number::Unsigned(a),
                                Number::Unsigned(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, u128);
                            }
                            (
                                NumericColumn::UInt16(values),
                                Number::Unsigned(a),
                                Number::Unsigned(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, u128);
                            }
                            (
                                NumericColumn::UInt32(values),
                                Number::Unsigned(a),
                                Number::Unsigned(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, u128);
                            }
                            (
                                NumericColumn::UInt64(values),
                                Number::Unsigned(a),
                                Number::Unsigned(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, u128);
                            }
                            (
                                NumericColumn::UInt128(values),
                                Number::Unsigned(a),
                                Number::Unsigned(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, u128);
                            }
                            (NumericColumn::Int8(values), Number::Signed(a), Number::Signed(b)) => {
                                return count_pair!(values, a, b, *first_op, *second_op, i128);
                            }
                            (
                                NumericColumn::Int16(values),
                                Number::Signed(a),
                                Number::Signed(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, i128);
                            }
                            (
                                NumericColumn::Int32(values),
                                Number::Signed(a),
                                Number::Signed(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, i128);
                            }
                            (
                                NumericColumn::Int64(values),
                                Number::Signed(a),
                                Number::Signed(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, i128);
                            }
                            (
                                NumericColumn::Int128(values),
                                Number::Signed(a),
                                Number::Signed(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, i128);
                            }
                            (
                                NumericColumn::Float32(values),
                                Number::Float(a),
                                Number::Float(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, f64);
                            }
                            (
                                NumericColumn::Float64(values),
                                Number::Float(a),
                                Number::Float(b),
                            ) => {
                                return count_pair!(values, a, b, *first_op, *second_op, f64);
                            }
                            _ => {}
                        }
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

fn order_numeric_conditions(conditions: &mut [CompiledPredicate]) {
    conditions.sort_by_key(|condition| match &condition.test {
        CompiledTest::Always(false) => 0,
        CompiledTest::Comparison {
            operator: Operator::Modulo(_) | Operator::ModuloNe(_),
            ..
        } => 4,
        CompiledTest::Comparison { .. }
        | CompiledTest::DistinctFrom { .. }
        | CompiledTest::NotBetween { .. } => 1,
        CompiledTest::InSet { .. } => 2,
        CompiledTest::In(values) | CompiledTest::NotIn(values) if values.len() <= 4 => 2,
        CompiledTest::In(_) | CompiledTest::NotIn(_) => 3,
        CompiledTest::Always(true) => 5,
    });
}

pub(super) fn compile_part_where(
    query: &Select,
    scan_columns: &mut Vec<String>,
    mut convert: impl FnMut(&str, &str) -> Result<Number, ExecutionError>,
) -> Result<Vec<Vec<CompiledPredicate>>, ExecutionError> {
    std::iter::once(query.predicates.as_slice())
        .chain(query.or_predicates.iter().map(Vec::as_slice))
        .map(|predicates| {
            let mut compiled = compile_part_predicates(predicates, scan_columns, &mut convert)?;
            order_numeric_conditions(&mut compiled);
            Ok(compiled)
        })
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
pub(super) fn monotonic_range_indices(
    branches: &[Vec<CompiledPredicate>],
    start: u64,
    rows: u64,
    step: u64,
) -> Option<(u64, u64)> {
    let [conjunction] = branches else { return None };
    if step == 0 {
        return None;
    }
    let first = u128::from(start);
    let stride = u128::from(step);
    let end = u128::from(rows);
    let mut lower = 0u128;
    let mut upper = end;
    for predicate in conjunction {
        let CompiledTest::Comparison {
            operator,
            target: Number::Unsigned(target),
        } = &predicate.test
        else {
            return None;
        };
        match operator {
            Operator::Eq => {
                if *target >= first && (*target - first) % stride == 0 {
                    let index = (*target - first) / stride;
                    lower = lower.max(index);
                    upper = upper.min(index.saturating_add(1));
                } else {
                    upper = 0;
                }
            }
            Operator::Lt => {
                upper = upper.min(if *target <= first {
                    0
                } else {
                    (*target - first).div_ceil(stride)
                });
            }
            Operator::Le => {
                upper = upper.min(if *target < first {
                    0
                } else {
                    ((*target - first) / stride).saturating_add(1)
                });
            }
            Operator::Gt => {
                lower = lower.max(if *target < first {
                    0
                } else {
                    ((*target - first) / stride).saturating_add(1)
                });
            }
            Operator::Ge => {
                lower = lower.max(if *target <= first {
                    0
                } else {
                    (*target - first).div_ceil(stride)
                });
            }
            Operator::Ne | Operator::Modulo(_) | Operator::ModuloNe(_) => return None,
        }
    }
    let lower = lower.min(end);
    let upper = upper.min(end).max(lower);
    Some((lower as u64, upper as u64))
}

/// Enumerate small finite matching sets across OR branches exactly.
/// Each disjunct needs a positive equality or IN seed. Unbounded branches
/// retain the normal executor rather than changing their semantics.
pub(super) fn sparse_generated_positions(
    branches: &[Vec<CompiledPredicate>],
    start: u64,
    rows: u64,
    step: u64,
) -> Option<Vec<u64>> {
    if step == 0 {
        return None;
    }
    let (start, step, rows) = (u128::from(start), u128::from(step), u128::from(rows));
    let mut indices = Vec::new();
    let mut examined = 0usize;
    for conjunction in branches {
        let mut seed = None;
        for predicate in conjunction {
            let candidates = match &predicate.test {
                CompiledTest::Comparison {
                    operator: Operator::Eq,
                    target: Number::Unsigned(value),
                } => Some(vec![*value]),
                CompiledTest::In(items) => Some(
                    items
                        .iter()
                        .filter_map(|item| {
                            if let Number::Unsigned(value) = item {
                                Some(*value)
                            } else {
                                None
                            }
                        })
                        .collect(),
                ),
                CompiledTest::InSet {
                    targets,
                    negated: false,
                } => Some(
                    targets
                        .iter()
                        .filter_map(|key| {
                            if let NumberKey::Unsigned(value) = key {
                                Some(*value)
                            } else {
                                None
                            }
                        })
                        .collect(),
                ),
                _ => None,
            };
            if candidates.is_some() {
                seed = candidates;
                break;
            }
        }
        let values: Vec<u128> = seed?;
        examined = examined.checked_add(values.len())?;
        if examined > 4096 {
            return None;
        }
        for value in values {
            if value < start || (value - start) % step != 0 {
                continue;
            }
            let index = (value - start) / step;
            if index < rows
                && conjunction
                    .iter()
                    .all(|predicate| predicate.test.matches(Number::Unsigned(value)))
            {
                indices.push(index as u64);
            }
        }
    }
    indices.sort_unstable();
    indices.dedup();
    Some(indices)
}

/// Return excluded positions for one bounded NOT IN predicate on generated rows.
pub(super) fn excluded_generated_positions(
    branches: &[Vec<CompiledPredicate>],
    start: u64,
    rows: u64,
    step: u64,
) -> Option<Vec<u64>> {
    let [branch] = branches else {
        return None;
    };
    let [predicate] = branch.as_slice() else {
        return None;
    };
    if step == 0 {
        return None;
    }
    let values: Vec<u128> = match &predicate.test {
        CompiledTest::NotIn(items) => items
            .iter()
            .filter_map(|value| {
                if let Number::Unsigned(value) = value {
                    Some(*value)
                } else {
                    None
                }
            })
            .collect(),
        CompiledTest::InSet {
            targets,
            negated: true,
        } => targets
            .iter()
            .filter_map(|key| {
                if let NumberKey::Unsigned(value) = key {
                    Some(*value)
                } else {
                    None
                }
            })
            .collect(),
        _ => return None,
    };
    if values.len() > 4096 {
        return None;
    }
    let (start, step, rows) = (u128::from(start), u128::from(step), u128::from(rows));
    let mut indices = Vec::new();
    for value in values {
        if value >= start && (value - start) % step == 0 {
            let index = (value - start) / step;
            if index < rows {
                indices.push(index as u64);
            }
        }
    }
    indices.sort_unstable();
    indices.dedup();
    Some(indices)
}

pub(super) fn count_monotonic_numbers(
    branches: &[Vec<CompiledPredicate>],
    start: u64,
    rows: u64,
    step: u64,
) -> Option<u64> {
    // Each OR equality names at most one position in a generated sequence.
    // Deduplicate positions so overlapping branches count only once.
    if branches.len() > 1 {
        if step == 0 {
            return None;
        }
        let mut positions = HashSet::new();
        for branch in branches {
            let [predicate] = branch.as_slice() else {
                return None;
            };
            let CompiledTest::Comparison {
                operator: Operator::Eq,
                target: Number::Unsigned(value),
            } = &predicate.test
            else {
                return None;
            };
            let first = u128::from(start);
            let increment = u128::from(step);
            if *value >= first {
                let delta = *value - first;
                if delta % increment == 0 {
                    let index = delta / increment;
                    if index < u128::from(rows) {
                        positions.insert(index);
                    }
                }
            }
        }
        return u64::try_from(positions.len()).ok();
    }
    let [conjunction] = branches else {
        return None;
    };
    if step == 0 {
        return None;
    }
    if let [predicate] = conjunction.as_slice() {
        if let CompiledTest::DistinctFrom {
            target: Number::Unsigned(target),
            negated,
        } = &predicate.test
        {
            let first = u128::from(start);
            let increment = u128::from(step);
            let exists = if *target < first {
                false
            } else {
                let delta = *target - first;
                delta % increment == 0 && delta / increment < u128::from(rows)
            };
            let matching = u64::from(exists);
            return Some(if *negated { matching } else { rows - matching });
        }
    }

    if let [predicate] = conjunction.as_slice() {
        if let CompiledTest::Always(value) = &predicate.test {
            return Some(if *value { rows } else { 0 });
        }
    }

    if let [predicate] = conjunction.as_slice() {
        if let CompiledTest::NotBetween {
            lower: Number::Unsigned(lower),
            upper: Number::Unsigned(upper),
        } = &predicate.test
        {
            if lower > upper {
                return Some(rows);
            }
            let first = u128::from(start);
            let increment = u128::from(step);
            let included_start = if *lower <= first {
                0
            } else {
                (*lower - first).div_ceil(increment)
            };
            let included_end = if *upper < first {
                0
            } else {
                ((*upper - first) / increment).saturating_add(1)
            };
            let hits = included_end
                .min(u128::from(rows))
                .saturating_sub(included_start.min(u128::from(rows)));
            return u64::try_from(u128::from(rows) - hits).ok();
        }
    }

    if let [predicate] = conjunction.as_slice() {
        let membership = match &predicate.test {
            CompiledTest::In(values) => Some((
                values
                    .iter()
                    .filter_map(|value| match value {
                        Number::Unsigned(value) => Some(*value),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                false,
            )),
            CompiledTest::NotIn(values) => Some((
                values
                    .iter()
                    .filter_map(|value| match value {
                        Number::Unsigned(value) => Some(*value),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                true,
            )),
            CompiledTest::InSet { targets, negated } => Some((
                targets
                    .iter()
                    .filter_map(|value| match value {
                        NumberKey::Unsigned(value) => Some(*value),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                *negated,
            )),
            _ => None,
        };
        if let Some((targets, negated)) = membership {
            let first = u128::from(start);
            let increment = u128::from(step);
            let mut selected = HashSet::new();
            for value in targets {
                if value >= first {
                    let delta = value - first;
                    if delta % increment == 0 {
                        let index = delta / increment;
                        if index < u128::from(rows) {
                            selected.insert(index);
                        }
                    }
                }
            }
            let hits = selected.len() as u64;
            return Some(if negated { rows - hits } else { hits });
        }
    }

    let start = u128::from(start);
    let step = u128::from(step);
    // For consecutive generated rows, a modulo filter is a periodic
    // arithmetic progression. Count matching positions in O(1) instead of
    // iterating over billions of generated values.
    if let [predicate] = conjunction.as_slice() {
        if let CompiledTest::Comparison {
            operator: Operator::Modulo(modulus) | Operator::ModuloNe(modulus),
            target: Number::Unsigned(target),
        } = &predicate.test
        {
            if step == 1 && *modulus != 0 {
                let divisor = u128::from(*modulus);
                let equal = if *target >= divisor {
                    0
                } else {
                    let first = (*target + divisor - (start % divisor)) % divisor;
                    if first >= u128::from(rows) {
                        0
                    } else {
                        1 + (u128::from(rows) - 1 - first) / divisor
                    }
                };
                let matches = if matches!(
                    predicate.test,
                    CompiledTest::Comparison {
                        operator: Operator::ModuloNe(_),
                        ..
                    }
                ) {
                    u128::from(rows) - equal
                } else {
                    equal
                };
                return u64::try_from(matches).ok();
            }
        }
    }
    let mut lower = 0u128;
    let mut upper = u128::from(rows);
    let mut excluded = Vec::with_capacity(conjunction.len().min(32));

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
    fn large_unsigned_sixteen_in_matches_hash_membership() {
        let values = (0..4096).map(|value| value as u16).collect::<Vec<_>>();
        let targets = (0..100).map(|value| Number::Unsigned(value * 2)).collect::<Vec<_>>();
        for negated in [false, true] {
            let predicate = CompiledPredicate {
                index: 0,
                test: compile_in_test(targets.clone(), negated),
            };
            let expected = values.iter().filter(|&&value| {
                (value < 200 && value % 2 == 0) != negated
            }).count();
            assert_eq!(
                count_batch_matches(&[NumericColumn::UInt16(values.clone())], &[vec![predicate]]),
                expected
            );
        }
    }

    #[test]
    fn compiles_fast_numeric_predicates_ahead_of_modulo() {
        let query = crate::parse_select(
            "SELECT number FROM numbers(100) WHERE number % 10 = 0 AND number < 50",
        )
        .unwrap();
        let branches = compile_numbers_predicates(&query).unwrap();
        assert_eq!(branches.len(), 1);
        assert!(matches!(
            &branches[0][0].test,
            CompiledTest::Comparison {
                operator: Operator::Lt,
                ..
            }
        ));
        assert!(number_matches_all(Number::Unsigned(40), &branches));
        assert!(!number_matches_all(Number::Unsigned(60), &branches));
    }

    #[test]
    fn direct_pair_counts_match_rowwise_across_numeric_types() {
        for (values, target_a, target_b) in [
            (
                NumericColumn::UInt16(vec![0, 1, 5, 10, 65535]),
                Number::Unsigned(1),
                Number::Unsigned(11),
            ),
            (
                NumericColumn::Int16(vec![-10, -1, 0, 5, 32767]),
                Number::Signed(-1),
                Number::Signed(6),
            ),
            (
                NumericColumn::Float32(vec![-1.0, 0.0, 0.5, 6.0, f32::NAN]),
                Number::Float(0.0),
                Number::Float(6.0),
            ),
        ] {
            let predicates = vec![vec![
                CompiledPredicate {
                    index: 0,
                    test: CompiledTest::Comparison {
                        operator: Operator::Ge,
                        target: target_a,
                    },
                },
                CompiledPredicate {
                    index: 0,
                    test: CompiledTest::Comparison {
                        operator: Operator::Lt,
                        target: target_b,
                    },
                },
            ]];
            let expected = (0..values.len())
                .filter(|&row| batch_matches(std::slice::from_ref(&values), row, &predicates))
                .count();
            assert_eq!(count_batch_matches(&[values], &predicates), expected);
        }
    }

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
mod stored_hashed_membership_count_tests {
    use super::*;

    #[test]
    fn hashed_membership_matches_row_by_row_for_wide_integer_columns() {
        let signed = vec![NumericColumn::Int64((-5_000..5_000).collect())];
        let unsigned = vec![NumericColumn::UInt64((0..10_000).collect())];
        for (batch, signed) in [(&signed, true), (&unsigned, false)] {
            for negated in [false, true] {
                let values: Vec<Number> = (0..32)
                    .map(|index| {
                        if signed {
                            Number::Signed(index - 12)
                        } else {
                            Number::Unsigned(index as u128)
                        }
                    })
                    .collect();
                let branches = vec![vec![CompiledPredicate {
                    index: 0,
                    test: compile_in_test(values, negated),
                }]];
                let expected = (0..batch[0].len())
                    .filter(|&row| batch_matches(batch, row, &branches))
                    .count();
                assert_eq!(count_batch_matches(batch, &branches), expected);
            }
        }
    }
}

#[cfg(test)]
mod stored_not_between_count_tests {
    use super::*;

    #[test]
    fn matches_row_wise_not_between_for_signed_and_nan_values() {
        let signed = vec![NumericColumn::Int64((-1_000..1_000).collect())];
        let floats = vec![NumericColumn::Float64(vec![
            f64::NAN,
            f64::NEG_INFINITY,
            -1.0,
            0.0,
            1.0,
            f64::INFINITY,
        ])];
        for (batch, lower, upper) in [
            (&signed, Number::Signed(-10), Number::Signed(10)),
            (&floats, Number::Float(-1.0), Number::Float(1.0)),
            (&floats, Number::Float(1.0), Number::Float(-1.0)),
        ] {
            let branches = vec![vec![CompiledPredicate {
                index: 0,
                test: CompiledTest::NotBetween { lower, upper },
            }]];
            let expected = (0..batch[0].len())
                .filter(|&row| batch_matches(batch, row, &branches))
                .count();
            assert_eq!(count_batch_matches(batch, &branches), expected);
        }
    }
}

#[cfg(test)]
mod stored_modulo_count_tests {
    use super::*;

    #[test]
    fn matches_signed_and_unsigned_modulo_counts() {
        let unsigned = vec![NumericColumn::UInt64((0..10_000).collect())];
        let signed = vec![NumericColumn::Int64((-5_000..5_000).collect())];
        for (batch, target, modulus) in [
            (&unsigned, Number::Unsigned(2), 7u64),
            (&signed, Number::Signed(-2), 7u64),
        ] {
            for negated in [false, true] {
                let branches = vec![vec![CompiledPredicate {
                    index: 0,
                    test: CompiledTest::Comparison {
                        operator: if negated {
                            Operator::ModuloNe(modulus)
                        } else {
                            Operator::Modulo(modulus)
                        },
                        target,
                    },
                }]];
                let expected = (0..batch[0].len())
                    .filter(|&row| batch_matches(batch, row, &branches))
                    .count();
                assert_eq!(count_batch_matches(batch, &branches), expected);
            }
        }
    }
}

#[cfg(test)]
mod stored_constant_count_tests {
    use super::*;

    #[test]
    fn counts_constant_predicates_without_per_row_dispatch() {
        let values = vec![NumericColumn::UInt64((0..100_000).collect())];
        for (value, expected) in [(true, 100_000), (false, 0)] {
            let branches = vec![vec![CompiledPredicate {
                index: 0,
                test: CompiledTest::Always(value),
            }]];
            assert_eq!(count_batch_matches(&values, &branches), expected);
        }
    }
}

#[cfg(test)]
mod generated_distinct_from_count_tests {
    use super::*;

    #[test]
    fn distinct_and_not_distinct_match_sequence_membership() {
        let count = |target, negated| {
            count_monotonic_numbers(
                &[vec![CompiledPredicate {
                    index: 0,
                    test: CompiledTest::DistinctFrom {
                        target: Number::Unsigned(target),
                        negated,
                    },
                }]],
                5,
                1_000_000_000,
                2,
            )
        };
        assert_eq!(count(7, true), Some(1));
        assert_eq!(count(7, false), Some(999_999_999));
        assert_eq!(count(8, true), Some(0));
        assert_eq!(count(8, false), Some(1_000_000_000));
    }
}

#[cfg(test)]
mod generated_constant_count_tests {
    use super::*;

    #[test]
    fn constant_predicates_do_not_iterate_billions_of_rows() {
        for (value, expected) in [(true, 1_000_000_000), (false, 0)] {
            assert_eq!(
                count_monotonic_numbers(
                    &[vec![CompiledPredicate {
                        index: 0,
                        test: CompiledTest::Always(value),
                    }]],
                    0,
                    1_000_000_000,
                    1
                ),
                Some(expected),
            );
        }
    }
}

#[cfg(test)]
mod generated_not_between_count_tests {
    use super::*;

    #[test]
    fn handles_billion_rows_step_and_reversed_bounds() {
        let count = |low, high, start, rows, step| {
            count_monotonic_numbers(
                &[vec![CompiledPredicate {
                    index: 0,
                    test: CompiledTest::NotBetween {
                        lower: Number::Unsigned(low),
                        upper: Number::Unsigned(high),
                    },
                }]],
                start,
                rows,
                step,
            )
        };
        assert_eq!(
            count(100, 899_999_999, 0, 1_000_000_000, 1),
            Some(100_000_100)
        );
        assert_eq!(count(3, 7, 1, 5, 2), Some(2));
        assert_eq!(count(9, 4, 0, 1_000_000_000, 1), Some(1_000_000_000));
    }
}

#[cfg(test)]
mod generated_membership_count_tests {
    use super::*;

    #[test]
    fn large_generated_membership_counts_only_the_list_values() {
        let small = vec![CompiledPredicate {
            index: 0,
            test: CompiledTest::In(vec![
                Number::Unsigned(1),
                Number::Unsigned(1),
                Number::Unsigned(99),
                Number::Unsigned(1_000_000_000),
            ]),
        }];
        assert_eq!(
            count_monotonic_numbers(&[small], 0, 1_000_000_000, 1),
            Some(2)
        );
        let set = (0..32).map(Number::Unsigned).collect();
        let large = vec![CompiledPredicate {
            index: 0,
            test: compile_in_test(set, true),
        }];
        assert_eq!(
            count_monotonic_numbers(&[large], 0, 1_000_000_000, 1),
            Some(999_999_968)
        );
    }
}

#[cfg(test)]
mod generated_or_equality_tests {
    use super::*;

    #[test]
    fn counts_unique_or_matches_on_a_billion_generated_rows() {
        let branch = |value| {
            vec![CompiledPredicate {
                index: 0,
                test: CompiledTest::Comparison {
                    operator: Operator::Eq,
                    target: Number::Unsigned(value),
                },
            }]
        };
        let branches = vec![
            branch(0),
            branch(4),
            branch(4),
            branch(999_999_999),
            branch(1_000_000_000),
        ];
        assert_eq!(
            count_monotonic_numbers(&branches, 0, 1_000_000_000, 1),
            Some(3)
        );
        assert_eq!(
            count_monotonic_numbers(&[branch(11), branch(15)], 1, 10, 2),
            Some(2)
        );
    }
}

#[cfg(test)]
mod generated_modulo_count_tests {
    use super::*;

    fn count(start: u64, rows: u64, modulus: u64, remainder: u128, negated: bool) -> u64 {
        let predicate = CompiledPredicate {
            index: 0,
            test: CompiledTest::Comparison {
                operator: if negated {
                    Operator::ModuloNe(modulus)
                } else {
                    Operator::Modulo(modulus)
                },
                target: Number::Unsigned(remainder),
            },
        };
        count_monotonic_numbers(&[vec![predicate]], start, rows, 1).unwrap()
    }

    #[test]
    fn modulo_count_skips_billion_row_iteration() {
        assert_eq!(count(0, 1_000_000_000, 7, 0, false), 142_857_143);
        assert_eq!(count(0, 1_000_000_000, 7, 0, true), 857_142_857);
    }

    #[test]
    fn modulo_count_handles_offsets_and_impossible_remainders() {
        assert_eq!(count(10, 20, 7, 0, false), 3);
        assert_eq!(count(10, 20, 7, 0, true), 17);
        assert_eq!(count(10, 20, 7, 7, false), 0);
        assert_eq!(count(10, 20, 7, 7, true), 20);
        assert_eq!(count(0, 0, 7, 0, false), 0);
    }
}

#[cfg(test)]
mod byte_in_dense_lookup_tests {
    use super::*;

    #[test]
    fn dense_in_matches_the_general_predicate_for_positive_and_negative_bytes() {
        for negated in [false, true] {
            for signed in [false, true] {
                let targets = if signed {
                    (-20i128..=20).map(NumberKey::Signed).collect()
                } else {
                    (50u128..=80).map(NumberKey::Unsigned).collect()
                };
                let test = CompiledTest::InSet { targets, negated };
                let batch = if signed {
                    vec![NumericColumn::Int8(
                        (-128i16..=127).map(|i| i as i8).collect::<Vec<_>>().repeat(4)
                    )]
                } else {
                    vec![NumericColumn::UInt8((0..=255).collect::<Vec<u8>>().repeat(4))]
                };
                let expected = (0..batch[0].len())
                    .filter(|&row| test.matches(super::super::number_at(&batch[0], row)))
                    .count();
                let predicate = CompiledPredicate { index: 0, test };
                assert_eq!(count_batch_matches(&batch, &[vec![predicate]]), expected);
            }
        }
    }
}

#[cfg(test)]
mod nan_not_between_semantics_tests {
    use super::*;

    #[test]
    fn nan_is_unordered_not_outside_an_interval() {
        let range = CompiledTest::NotBetween {
            lower: Number::Float(-1.0), upper: Number::Float(1.0),
        };
        assert!(!range.matches(Number::Float(f64::NAN)));
        assert!(!range.matches(Number::Float(0.0)));
        assert!(range.matches(Number::Float(3.0)));
        let invalid = CompiledTest::NotBetween {
            lower: Number::Float(f64::NAN), upper: Number::Float(3.0),
        };
        assert!(!invalid.matches(Number::Float(7.0)));
    }

    #[test]
    fn vectorized_not_between_agrees_with_unordered_rowwise_semantics() {
        let column = NumericColumn::Float64(vec![f64::NAN, -2.0, 0.0, 2.0]);
        let branches = vec![vec![CompiledPredicate {
            index: 0, test: CompiledTest::NotBetween {
                lower: Number::Float(-1.0), upper: Number::Float(1.0)
            }
        }]];
        assert_eq!(count_batch_matches(std::slice::from_ref(&column), &branches), 2);
        assert_eq!((0..column.len()).filter(|&i| {
            batch_matches(std::slice::from_ref(&column), i, &branches)
        }).count(), 2);
    }
}
