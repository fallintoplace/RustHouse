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

fn compile_part_predicates(
    predicates: &[Predicate],
    scan_columns: &mut Vec<String>,
    mut convert: impl FnMut(&str, &str) -> Result<Number, ExecutionError>,
) -> Result<Vec<CompiledPredicate>, ExecutionError> {
    let mut compiled = Vec::with_capacity(predicates.len());
    for predicate in predicates {
        let column = predicate.column();
        let test = match predicate {
            Predicate::Comparison {
                operator, value, ..
            } => {
                let target = convert(column, value)?;
                if matches!(*operator, Operator::Modulo(_) | Operator::ModuloNe(_))
                    && matches!(target, Number::Float(_))
                {
                    return Err(ExecutionError::Invalid(
                        "modulo requires integer columns".into(),
                    ));
                }
                CompiledTest::Comparison {
                    operator: *operator,
                    target,
                }
            }
            Predicate::In { values, .. } => compile_in_test(
                values
                    .iter()
                    .map(|value| convert(column, value))
                    .collect::<Result<Vec<_>, _>>()?,
                false,
            ),
            Predicate::NotIn { values, .. } => compile_in_test(
                values
                    .iter()
                    .map(|value| convert(column, value))
                    .collect::<Result<Vec<_>, _>>()?,
                true,
            ),
            Predicate::IsNull { .. } => {
                return Err(ExecutionError::Invalid("IS NULL requires a typed scan".into()));
            }
            Predicate::NotBetween { lower, upper, .. } => CompiledTest::NotBetween {
                lower: convert(column, lower)?,
                upper: convert(column, upper)?,
            },
        };
        let index = if let Some(index) = scan_columns
            .iter()
            .position(|candidate| candidate.as_str() == column)
        {
            index
        } else {
            scan_columns.push(column.to_owned());
            scan_columns.len() - 1
        };
        compiled.push(CompiledPredicate { index, test });
    }
    Ok(compiled)
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

pub(super) fn compile_part_where(
    query: &Select,
    scan_columns: &mut Vec<String>,
    mut convert: impl FnMut(&str, &str) -> Result<Number, ExecutionError>,
) -> Result<Vec<Vec<CompiledPredicate>>, ExecutionError> {
    let mut result = vec![compile_part_predicates(
        &query.predicates,
        scan_columns,
        |column, value| convert(column, value),
    )?];
    for predicates in &query.or_predicates {
        result.push(compile_part_predicates(
            predicates,
            scan_columns,
            |column, value| convert(column, value),
        )?);
    }
    Ok(result)
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

fn compare<T: PartialOrd>(value: T, target: T, operator: Operator) -> bool {
    match operator {
        Operator::Eq => value == target,
        Operator::Ne => value != target,
        Operator::Lt => value < target,
        Operator::Le => value <= target,
        Operator::Gt => value > target,
        Operator::Ge => value >= target,
        Operator::Modulo(_) | Operator::ModuloNe(_) => {
            unreachable!("modulo handled before comparison")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
