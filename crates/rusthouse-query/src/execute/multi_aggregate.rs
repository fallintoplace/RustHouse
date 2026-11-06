mod arithmetic;

use self::arithmetic::*;

use super::typed::{self, Cell};
use super::{
    AggregateState, BitmapDistinct32, ExecutionError, Number, NumberKey, checked_aggregate_sum,
    number_at,
};
use crate::{AggregateItem, ScalarExpr, Select};
use rusthouse_storage::{
    NumericColumn, PartMetadata, TypedColumn, TypedQueryReader, inspect_query_source,
};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

const MAX_DISTINCT_VALUES: usize = 100_000;
const MAX_DISTINCT_TEXT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
struct DistinctValues {
    other: HashSet<typed::Key>,
    text: HashSet<Vec<u8>>,
    uuids: HashSet<[u8; 16]>,
    retained_text_bytes: usize,
    // Multi-metric queries share the exact UInt32/Int32 bitmap fast path.
    bitmap32: Option<BitmapDistinct32>,
}

impl DistinctValues {
    fn len(&self) -> usize {
        self.other.len()
            + self.text.len()
            + self.uuids.len()
            + self
                .bitmap32
                .as_ref()
                .map_or(0, |bitmap| bitmap.len() as usize)
    }

    fn insert_key(&mut self, key: typed::Key) -> Result<(), ExecutionError> {
        if self.len() >= MAX_DISTINCT_VALUES && !self.other.contains(&key) {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) supports at most 100000 values".into(),
            ));
        }
        self.other.insert(key);
        Ok(())
    }

    fn insert_text(&mut self, value: &[u8]) -> Result<(), ExecutionError> {
        // Avoid allocating for duplicate String keys across millions of rows.
        if self.text.contains(value) {
            return Ok(());
        }
        if self.len() >= MAX_DISTINCT_VALUES {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) supports at most 100000 values".into(),
            ));
        }
        let additional = value
            .len()
            .checked_add(std::mem::size_of::<Vec<u8>>() + 2 * std::mem::size_of::<usize>())
            .ok_or_else(|| {
                ExecutionError::Invalid("multi-aggregate DISTINCT String memory overflow".into())
            })?;
        let next = self
            .retained_text_bytes
            .checked_add(additional)
            .ok_or_else(|| {
                ExecutionError::Invalid("multi-aggregate DISTINCT String memory overflow".into())
            })?;
        if next > MAX_DISTINCT_TEXT_BYTES {
            return Err(ExecutionError::Invalid(
                "multi-aggregate COUNT(DISTINCT) String keys exceed 64 MiB".into(),
            ));
        }
        self.text.insert(value.to_vec());
        self.retained_text_bytes = next;
        Ok(())
    }

    fn insert_uuid(&mut self, value: [u8; 16]) -> Result<(), ExecutionError> {
        if self.len() >= MAX_DISTINCT_VALUES && !self.uuids.contains(&value) {
            return Err(ExecutionError::Invalid(
                "COUNT(DISTINCT) supports at most 100000 values".into(),
            ));
        }
        self.uuids.insert(value);
        Ok(())
    }

    fn insert_column(&mut self, column: &TypedColumn, row: usize) -> Result<bool, ExecutionError> {
        if let Some(bitmap) = self.bitmap32.as_mut() {
            let value = match column {
                TypedColumn::Numeric(NumericColumn::UInt32(values)) => Some(values[row]),
                TypedColumn::Numeric(NumericColumn::Int32(values)) => Some(values[row] as u32),
                TypedColumn::NullableNumeric {
                    values: NumericColumn::UInt32(values),
                    nulls,
                } => (nulls[row] == 0).then_some(values[row]),
                TypedColumn::NullableNumeric {
                    values: NumericColumn::Int32(values),
                    nulls,
                } => (nulls[row] == 0).then_some(values[row] as u32),
                _ => {
                    return Err(ExecutionError::Invalid(
                        "32-bit distinct bitmap requires a 32-bit integer column".into(),
                    ));
                }
            };
            if let Some(key) = value {
                bitmap.insert(key)?;
            }
            return Ok(true);
        }
        match column {
            TypedColumn::Text(values) => {
                if let Some(value) = values.get(row) {
                    self.insert_text(value)?;
                }
            }
            TypedColumn::NullableText { values, nulls } => {
                if nulls[row] == 0 {
                    if let Some(value) = values.get(row) {
                        self.insert_text(value)?;
                    }
                }
            }
            TypedColumn::Uuid(values) => self.insert_uuid(*values[row].as_bytes())?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

enum State {
    Count(u64),
    Distinct(DistinctValues),
    Numeric(AggregateState),
}

// Resolve column names once per query rather than for every aggregate and row.
pub(super) enum BoundExpr {
    Column(usize),
    TextLength(usize),
    TextLiteral(Vec<u8>),
    TextConcat(Vec<Self>),
    TextSubstring {
        value: Box<Self>,
        start: usize,
        length: Option<usize>,
    },
    Round {
        value: Box<Self>,
        precision: i32,
    },
    Integer(i64),
    Float(f64),
    // Fast path for the affine expression family in ClickBench Q30.
    AffineColumn {
        index: usize,
        constant: i64,
        subtract: bool,
    },
    Add(Box<Self>, Box<Self>),
    Subtract(Box<Self>, Box<Self>),
    Multiply(Box<Self>, Box<Self>),
    Divide(Box<Self>, Box<Self>),
    Modulo(Box<Self>, Box<Self>),
    Compare(Box<Self>, crate::Operator, Box<Self>),
    IsNull {
        index: usize,
        negated: bool,
    },
    TextCompare {
        index: usize,
        operator: crate::Operator,
        value: Vec<u8>,
    },
    TextLike {
        index: usize,
        pattern: typed::TextPattern,
        negated: bool,
        case_insensitive: bool,
    },
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
    Case {
        branches: Vec<(Self, Self)>,
        else_expr: Option<Box<Self>>,
    },
}

fn affine_sum_plan(
    items: &[AggregateItem],
    arguments: &[Option<BoundExpr>],
) -> Option<(usize, Vec<i64>)> {
    if items.is_empty() || items.len() != arguments.len() {
        return None;
    }
    let mut source = None;
    let mut constants = Vec::with_capacity(items.len());
    for (item, argument) in items.iter().zip(arguments) {
        if item.function != Some(crate::AggregateFunction::Sum) || item.distinct {
            return None;
        }
        let (index, constant) = match argument.as_ref()? {
            BoundExpr::Column(index) => (*index, 0),
            BoundExpr::AffineColumn {
                index,
                constant,
                subtract: false,
            } if *constant >= 0 => (*index, *constant),
            _ => return None,
        };
        if source.is_some_and(|source| source != index) {
            return None;
        }
        source = Some(index);
        constants.push(constant);
    }
    source.map(|source| (source, constants))
}

fn affine_sum_range_is_safe(data_type: &str, rows: u64, max_constant: i64) -> bool {
    let max_abs_value = match typed::base_type(data_type) {
        "UInt8" => u128::from(u8::MAX),
        "UInt16" => u128::from(u16::MAX),
        "UInt32" => u128::from(u32::MAX),
        "Int8" => 1_u128 << 7,
        "Int16" => 1_u128 << 15,
        "Int32" => 1_u128 << 31,
        _ => return false,
    };
    let max_result = max_abs_value.checked_add(max_constant as u128);
    max_result
        .and_then(|value| value.checked_mul(u128::from(rows)))
        .is_some_and(|bound| bound <= i128::MAX as u128)
}

fn accumulate_affine_base(
    column: &TypedColumn,
    base_sum: &mut Option<Number>,
    non_null_rows: &mut u64,
) -> Result<(), ExecutionError> {
    let (values, nulls) = match column {
        TypedColumn::Numeric(values) => (values, None),
        TypedColumn::NullableNumeric { values, nulls } => (values, Some(nulls.as_slice())),
        _ => {
            return Err(ExecutionError::Invalid(
                "aggregate expression requires numeric columns".into(),
            ));
        }
    };
    for row in 0..values.len() {
        if nulls.is_some_and(|nulls| nulls[row] != 0) {
            continue;
        }
        let next = number_at(values, row);
        *base_sum = Some(match *base_sum {
            Some(current) => checked_aggregate_sum(current, next)?,
            None => next,
        });
        *non_null_rows = non_null_rows
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("aggregate row count overflow".into()))?;
    }
    Ok(())
}

fn affine_sum_values(
    base_sum: Option<Number>,
    non_null_rows: u64,
    constants: &[i64],
) -> Result<Vec<Option<Number>>, ExecutionError> {
    constants
        .iter()
        .map(|&constant| {
            let Some(base_sum) = base_sum else {
                return Ok(None);
            };
            let adjustment = i128::from(constant)
                .checked_mul(i128::from(non_null_rows))
                .ok_or_else(|| ExecutionError::Invalid("aggregate expression overflow".into()))?;
            checked_binary(base_sum, Number::Signed(adjustment), false).map(Some)
        })
        .collect()
}

fn affine_sum_states(
    part: &Path,
    metadata: &PartMetadata,
    names: &[String],
    types: &HashMap<String, String>,
    items: &[AggregateItem],
    arguments: &[Option<BoundExpr>],
) -> Result<Option<Vec<State>>, ExecutionError> {
    let Some((index, constants)) = affine_sum_plan(items, arguments) else {
        return Ok(None);
    };
    let Some(column) = names.get(index) else {
        return Ok(None);
    };
    let Some(data_type) = types.get(column) else {
        return Ok(None);
    };
    let max_constant = constants.iter().copied().max().unwrap_or(0);
    if !affine_sum_range_is_safe(data_type, metadata.rows, max_constant) {
        return Ok(None);
    }

    let mut reader =
        TypedQueryReader::open_with_metadata(part, std::slice::from_ref(column), metadata)?;
    let mut base_sum = None;
    let mut non_null_rows = 0u64;
    let mut scanned_rows = 0u64;
    while let Some(batch) = reader.next_batch()? {
        let source = batch
            .first()
            .ok_or_else(|| ExecutionError::Invalid("aggregate source is empty".into()))?;
        scanned_rows = scanned_rows
            .checked_add(source.len() as u64)
            .ok_or_else(|| ExecutionError::Invalid("aggregate row count overflow".into()))?;
        accumulate_affine_base(source, &mut base_sum, &mut non_null_rows)?;
    }
    if scanned_rows != metadata.rows {
        return Err(ExecutionError::Invalid(
            "aggregate input row count changed during scan".into(),
        ));
    }
    let values = affine_sum_values(base_sum, non_null_rows, &constants)?;
    let mut states = items
        .iter()
        .map(|item| State::Numeric(AggregateState::new(item.function.expect("SUM plan"))))
        .collect::<Vec<_>>();
    for (state, value) in states.iter_mut().zip(values) {
        if let (State::Numeric(state), Some(value)) = (state, value) {
            state.update_sum(value)?;
        }
    }
    Ok(Some(states))
}

impl BoundExpr {
    pub(super) fn bind(
        expression: &ScalarExpr,
        indices: &HashMap<String, usize>,
    ) -> Result<Self, ExecutionError> {
        let index = |name: &str| {
            indices
                .get(name)
                .copied()
                .ok_or_else(|| ExecutionError::Invalid(format!("unknown aggregate column: {name}")))
        };
        Ok(match expression {
            ScalarExpr::Case {
                branches,
                else_expr,
            } => Self::Case {
                branches: branches
                    .iter()
                    .map(|(condition, value)| {
                        Ok((Self::bind(condition, indices)?, Self::bind(value, indices)?))
                    })
                    .collect::<Result<Vec<_>, ExecutionError>>()?,
                else_expr: else_expr
                    .as_ref()
                    .map(|value| Self::bind(value, indices).map(Box::new))
                    .transpose()?,
            },
            ScalarExpr::Column(name) => Self::Column(index(name)?),
            ScalarExpr::TextLiteral(value) => Self::TextLiteral(value.as_bytes().to_vec()),
            ScalarExpr::TextConcat(arguments) => Self::TextConcat(
                arguments
                    .iter()
                    .map(|argument| Self::bind(argument, indices))
                    .collect::<Result<Vec<_>, ExecutionError>>()?,
            ),
            ScalarExpr::TextSubstring {
                value,
                start,
                length,
            } => Self::TextSubstring {
                value: Box::new(Self::bind(value, indices)?),
                start: *start,
                length: *length,
            },
            ScalarExpr::Round { value, precision } => Self::Round {
                value: Box::new(Self::bind(value, indices)?),
                precision: *precision,
            },
            ScalarExpr::IsNull { column, negated } => Self::IsNull {
                index: index(column)?,
                negated: *negated,
            },
            ScalarExpr::TextLength(name) => Self::TextLength(index(name)?),
            ScalarExpr::TextCompare {
                column,
                operator,
                value,
            } => Self::TextCompare {
                index: index(column)?,
                operator: *operator,
                value: value.as_bytes().to_vec(),
            },
            ScalarExpr::TextLike {
                column,
                pattern,
                negated,
                case_insensitive,
            } => Self::TextLike {
                index: index(column)?,
                pattern: typed::compile_text_pattern(pattern)?,
                negated: *negated,
                case_insensitive: *case_insensitive,
            },
            ScalarExpr::And(left, right) => Self::And(
                Box::new(Self::bind(left, indices)?),
                Box::new(Self::bind(right, indices)?),
            ),
            ScalarExpr::Or(left, right) => Self::Or(
                Box::new(Self::bind(left, indices)?),
                Box::new(Self::bind(right, indices)?),
            ),
            ScalarExpr::Integer(number) => Self::Integer(*number),
            ScalarExpr::Decimal(number) => {
                let value = number.parse::<f64>().map_err(|_| {
                    ExecutionError::Invalid("invalid decimal expression literal".into())
                })?;
                if !value.is_finite() {
                    return Err(ExecutionError::Invalid("non-finite decimal literal".into()));
                }
                Self::Float(value)
            }
            ScalarExpr::Add(left, right) => {
                if let (ScalarExpr::Column(name), ScalarExpr::Integer(constant)) =
                    (left.as_ref(), right.as_ref())
                {
                    Self::AffineColumn {
                        index: index(name)?,
                        constant: *constant,
                        subtract: false,
                    }
                } else {
                    Self::Add(
                        Box::new(Self::bind(left, indices)?),
                        Box::new(Self::bind(right, indices)?),
                    )
                }
            }
            ScalarExpr::Subtract(left, right) => {
                if let (ScalarExpr::Column(name), ScalarExpr::Integer(constant)) =
                    (left.as_ref(), right.as_ref())
                {
                    Self::AffineColumn {
                        index: index(name)?,
                        constant: *constant,
                        subtract: true,
                    }
                } else {
                    Self::Subtract(
                        Box::new(Self::bind(left, indices)?),
                        Box::new(Self::bind(right, indices)?),
                    )
                }
            }
            ScalarExpr::Multiply(left, right) => Self::Multiply(
                Box::new(Self::bind(left, indices)?),
                Box::new(Self::bind(right, indices)?),
            ),
            ScalarExpr::Divide(left, right) => Self::Divide(
                Box::new(Self::bind(left, indices)?),
                Box::new(Self::bind(right, indices)?),
            ),
            ScalarExpr::Modulo(left, right) => Self::Modulo(
                Box::new(Self::bind(left, indices)?),
                Box::new(Self::bind(right, indices)?),
            ),
            ScalarExpr::Compare(left, operator, right) => Self::Compare(
                Box::new(Self::bind(left, indices)?),
                *operator,
                Box::new(Self::bind(right, indices)?),
            ),
        })
    }
}

fn numeric_argument_at(
    batch: &[TypedColumn],
    index: usize,
    row: usize,
) -> Result<Option<Number>, ExecutionError> {
    // Number extraction does not require allocating a String/UUID Cell.
    match &batch[index] {
        TypedColumn::Numeric(values) => Ok(Some(number_at(values, row))),
        TypedColumn::NullableNumeric { values, nulls } => {
            if nulls[row] != 0 {
                Ok(None)
            } else {
                Ok(Some(number_at(values, row)))
            }
        }
        _ => Err(ExecutionError::Invalid(
            "aggregate expression requires numeric columns".into(),
        )),
    }
}

/// Compare a UInt128 against an IEEE Float64 without rounding the integer
/// through Float64 first. Conversions below are exact for in-range whole
/// Float64 values; fractional portions are resolved after integer comparison.
fn compare_unsigned_float(integer: u128, float: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if float.is_nan() {
        return None;
    }
    if float < 0.0 {
        return Some(Ordering::Greater);
    }
    if float.is_infinite() || float >= 2.0_f64.powi(128) {
        return Some(Ordering::Less);
    }
    let whole = float.trunc() as u128;
    Some(integer.cmp(&whole).then_with(|| {
        if float.fract() > 0.0 {
            Ordering::Less
        } else {
            Ordering::Equal
        }
    }))
}

fn compare_signed_float(integer: i128, float: f64) -> Option<std::cmp::Ordering> {
    if integer >= 0 {
        compare_unsigned_float(integer as u128, float)
    } else if float >= 0.0 {
        Some(std::cmp::Ordering::Less)
    } else {
        compare_unsigned_float(integer.unsigned_abs(), -float).map(std::cmp::Ordering::reverse)
    }
}

/// SQL expression comparisons must compare numeric values, not the
/// internal Number enum variant order used by mixed-type sort keys.
pub(super) fn compare_expression_numbers(
    left: Number,
    right: Number,
) -> Option<std::cmp::Ordering> {
    match (left, right) {
        (Number::Unsigned(left), Number::Signed(right)) => Some(match u128::try_from(right) {
            Ok(right) => left.cmp(&right),
            Err(_) => std::cmp::Ordering::Greater,
        }),
        (Number::Signed(left), Number::Unsigned(right)) => Some(match u128::try_from(left) {
            Ok(left) => left.cmp(&right),
            Err(_) => std::cmp::Ordering::Less,
        }),
        (Number::Float(left), Number::Float(right)) => left.partial_cmp(&right),
        (Number::Unsigned(left), Number::Float(right)) => compare_unsigned_float(left, right),
        (Number::Signed(left), Number::Float(right)) => compare_signed_float(left, right),
        (Number::Float(left), Number::Unsigned(right)) => {
            compare_unsigned_float(right, left).map(std::cmp::Ordering::reverse)
        }
        (Number::Float(left), Number::Signed(right)) => {
            compare_signed_float(right, left).map(std::cmp::Ordering::reverse)
        }
        _ => Some(super::compare_numbers(left, right)),
    }
}

// Evaluate the numeric arithmetic subset against an already joined row.
fn round_number(value: Number, precision: i32) -> Result<Number, ExecutionError> {
    if precision >= 0 {
        if !matches!(value, Number::Float(_)) {
            return Ok(value);
        }
        let Number::Float(value) = value else {
            unreachable!();
        };
        let factor = 10_f64.powi(precision);
        let rounded = if factor.is_finite() {
            let scaled = value * factor;
            if scaled.is_finite() {
                scaled.round() / factor
            } else {
                value
            }
        } else {
            value
        };
        return rounded
            .is_finite()
            .then_some(Number::Float(rounded))
            .ok_or_else(|| ExecutionError::Invalid("non-finite ROUND result".into()));
    }

    let scale = precision.unsigned_abs();
    let rounded = match value {
        Number::Signed(value) => {
            let Some(divisor) = 10_u128.checked_pow(scale) else {
                return Ok(Number::Signed(0));
            };
            let magnitude = value.unsigned_abs();
            let units = magnitude / divisor;
            let remainder = magnitude % divisor;
            let rounded_units = units
                .checked_add(u128::from(remainder >= divisor.div_ceil(2)))
                .ok_or_else(|| ExecutionError::Invalid("ROUND result overflow".into()))?;
            let rounded = rounded_units
                .checked_mul(divisor)
                .ok_or_else(|| ExecutionError::Invalid("ROUND result overflow".into()))?;
            if value < 0 {
                if rounded == 1_u128 << 127 {
                    Number::Signed(i128::MIN)
                } else {
                    let rounded = i128::try_from(rounded)
                        .map_err(|_| ExecutionError::Invalid("ROUND result overflow".into()))?;
                    Number::Signed(-rounded)
                }
            } else {
                Number::Signed(
                    i128::try_from(rounded)
                        .map_err(|_| ExecutionError::Invalid("ROUND result overflow".into()))?,
                )
            }
        }
        Number::Unsigned(value) => {
            let Some(divisor) = 10_u128.checked_pow(scale) else {
                return Ok(Number::Unsigned(0));
            };
            let units = value / divisor;
            let remainder = value % divisor;
            let rounded_units = units
                .checked_add(u128::from(remainder >= divisor.div_ceil(2)))
                .ok_or_else(|| ExecutionError::Invalid("ROUND result overflow".into()))?;
            Number::Unsigned(
                rounded_units
                    .checked_mul(divisor)
                    .ok_or_else(|| ExecutionError::Invalid("ROUND result overflow".into()))?,
            )
        }
        Number::Float(value) => {
            let factor = 10_f64.powi(precision.saturating_neg());
            let rounded = if factor.is_finite() {
                (value / factor).round() * factor
            } else {
                0.0
            };
            if !rounded.is_finite() {
                return Err(ExecutionError::Invalid("non-finite ROUND result".into()));
            }
            Number::Float(rounded)
        }
    };
    Ok(rounded)
}

pub(super) fn eval_row_arithmetic(
    expression: &BoundExpr,
    row: &[Cell],
) -> Result<Option<Number>, ExecutionError> {
    match expression {
        BoundExpr::TextLiteral(_) | BoundExpr::TextConcat(_) | BoundExpr::TextSubstring { .. } => {
            Err(ExecutionError::Invalid(
                "text scalar expression cannot be used as a numeric expression".into(),
            ))
        }
        BoundExpr::Round { value, precision } => match eval_row_arithmetic(value, row)? {
            Some(value) => round_number(value, *precision).map(Some),
            None => Ok(None),
        },
        BoundExpr::Case {
            branches,
            else_expr,
        } => {
            for (condition, value) in branches {
                let matched = match eval_row_arithmetic(condition, row)? {
                    Some(Number::Unsigned(value)) => value != 0,
                    Some(Number::Signed(value)) => value != 0,
                    Some(Number::Float(value)) => value != 0.0,
                    None => false,
                };
                if matched {
                    return eval_row_arithmetic(value, row);
                }
            }
            match else_expr {
                Some(value) => eval_row_arithmetic(value, row),
                None => Ok(None),
            }
        }
        BoundExpr::TextCompare {
            index,
            operator,
            value,
        } => match &row[*index] {
            Cell::Null => Ok(None),
            Cell::Text(source) => {
                let order = source.cmp(value);
                let result = match operator {
                    crate::Operator::Eq => order.is_eq(),
                    crate::Operator::Ne => order.is_ne(),
                    crate::Operator::Lt => order.is_lt(),
                    crate::Operator::Le => order.is_le(),
                    crate::Operator::Gt => order.is_gt(),
                    crate::Operator::Ge => order.is_ge(),
                    _ => {
                        return Err(ExecutionError::Invalid(
                            "unsupported text comparison".into(),
                        ));
                    }
                };
                Ok(Some(Number::Unsigned(u128::from(result))))
            }
            _ => Err(ExecutionError::Invalid(
                "text expression requires a String column".into(),
            )),
        },
        BoundExpr::TextLike {
            index,
            pattern,
            negated,
            case_insensitive,
        } => match &row[*index] {
            Cell::Null => Ok(None),
            Cell::Text(source) => Ok(Some(Number::Unsigned(u128::from(
                typed::text_pattern_matches(source, pattern, *case_insensitive) != *negated,
            )))),
            _ => Err(ExecutionError::Invalid(
                "LIKE expression requires a String column".into(),
            )),
        },
        BoundExpr::TextLength(index) => match &row[*index] {
            Cell::Null => Ok(None),
            Cell::Text(source) => Ok(Some(Number::Unsigned(source.len() as u128))),
            _ => Err(ExecutionError::Invalid(
                "length requires a String column".into(),
            )),
        },
        BoundExpr::Compare(left, operator, right) => {
            let Some(left) = eval_row_arithmetic(left, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_row_arithmetic(right, row)? else {
                return Ok(None);
            };
            let order = compare_expression_numbers(left, right);
            let result = match operator {
                crate::Operator::Eq => order.is_some_and(|order| order.is_eq()),
                crate::Operator::Ne => order.is_none_or(|order| order.is_ne()),
                crate::Operator::Lt => order.is_some_and(|order| order.is_lt()),
                crate::Operator::Le => order.is_some_and(|order| order.is_le()),
                crate::Operator::Gt => order.is_some_and(|order| order.is_gt()),
                crate::Operator::Ge => order.is_some_and(|order| order.is_ge()),
                _ => {
                    return Err(ExecutionError::Invalid(
                        "unsupported numeric comparison".into(),
                    ));
                }
            };
            Ok(Some(Number::Unsigned(u128::from(result))))
        }
        BoundExpr::IsNull { index, negated } => Ok(Some(Number::Unsigned(u128::from(
            matches!(row.get(*index), Some(Cell::Null)) == !*negated,
        )))),
        BoundExpr::And(left, right) | BoundExpr::Or(left, right) => {
            let as_bool = |value: Number| match value {
                Number::Unsigned(v) => v != 0,
                Number::Signed(v) => v != 0,
                Number::Float(v) => v != 0.0,
            };
            let is_and = matches!(expression, BoundExpr::And(..));
            let left = eval_row_arithmetic(left, row)?.map(as_bool);
            if left == Some(!is_and) {
                return Ok(Some(Number::Unsigned(u128::from(!is_and))));
            }
            let right = eval_row_arithmetic(right, row)?.map(as_bool);
            let result = match (left, right) {
                (Some(a), Some(b)) => Some(if is_and { a && b } else { a || b }),
                (Some(a), None) => {
                    if a != is_and {
                        Some(a)
                    } else {
                        None
                    }
                }
                (None, Some(b)) => {
                    if b != is_and {
                        Some(b)
                    } else {
                        None
                    }
                }
                (None, None) => None,
            };
            Ok(result.map(|value| Number::Unsigned(u128::from(value))))
        }
        BoundExpr::Column(index) => match row.get(*index) {
            Some(Cell::Numeric(value)) => Ok(Some(*value)),
            Some(Cell::Null) => Ok(None),
            _ => Err(ExecutionError::Invalid(
                "aggregate expression requires numeric columns".into(),
            )),
        },
        BoundExpr::Integer(value) => Ok(Some(Number::Signed(i128::from(*value)))),
        BoundExpr::Float(value) => Ok(Some(Number::Float(*value))),
        BoundExpr::AffineColumn {
            index,
            constant,
            subtract,
        } => match eval_row_arithmetic(&BoundExpr::Column(*index), row)? {
            Some(value) => {
                checked_binary(value, Number::Signed(i128::from(*constant)), *subtract).map(Some)
            }
            None => Ok(None),
        },
        BoundExpr::Add(left, right)
        | BoundExpr::Subtract(left, right)
        | BoundExpr::Multiply(left, right)
        | BoundExpr::Divide(left, right)
        | BoundExpr::Modulo(left, right) => {
            let Some(left) = eval_row_arithmetic(left, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_row_arithmetic(right, row)? else {
                return Ok(None);
            };
            match expression {
                BoundExpr::Add(..) => checked_binary(left, right, false).map(Some),
                BoundExpr::Subtract(..) => checked_binary(left, right, true).map(Some),
                BoundExpr::Multiply(..) => checked_multiply(left, right).map(Some),
                BoundExpr::Modulo(..) => checked_remainder(left, right).map(Some),
                BoundExpr::Divide(..) => {
                    let as_float = |value| match value {
                        Number::Unsigned(value) => value as f64,
                        Number::Signed(value) => value as f64,
                        Number::Float(value) => value,
                    };
                    let divisor = as_float(right);
                    if divisor == 0.0 {
                        return Err(ExecutionError::Invalid(
                            "division by zero in aggregate expression".into(),
                        ));
                    }
                    let result = as_float(left) / divisor;
                    if !result.is_finite() {
                        return Err(ExecutionError::Invalid(
                            "non-finite aggregate division".into(),
                        ));
                    }
                    Ok(Some(Number::Float(result)))
                }
                _ => unreachable!(),
            }
        }
    }
}

/// Evaluate a scalar projection while preserving either text or numeric output.
pub(super) fn eval_row_value(expression: &BoundExpr, row: &[Cell]) -> Result<Cell, ExecutionError> {
    match expression {
        BoundExpr::Column(index) => row.get(*index).cloned().ok_or_else(|| {
            ExecutionError::Invalid("scalar projection references an unknown column".into())
        }),
        BoundExpr::TextLiteral(value) => Ok(Cell::Text(value.clone())),
        BoundExpr::TextConcat(arguments) => {
            let mut output = Vec::new();
            for argument in arguments {
                match eval_row_value(argument, row)? {
                    Cell::Text(value) => output.extend_from_slice(&value),
                    Cell::Uuid(value) => output.extend_from_slice(value.as_bytes()),
                    // DuckDB's CONCAT skips NULL arguments and yields an empty
                    // string when every argument is NULL.
                    Cell::Null => {}
                    Cell::Numeric(_) => {
                        return Err(ExecutionError::Invalid(
                            "CONCAT requires String arguments".into(),
                        ));
                    }
                }
            }
            Ok(Cell::Text(output))
        }
        BoundExpr::TextSubstring {
            value,
            start,
            length,
        } => match eval_row_value(value, row)? {
            Cell::Text(source) => {
                let source = std::str::from_utf8(&source).map_err(|_| {
                    ExecutionError::Invalid("SUBSTRING input is not valid UTF-8".into())
                })?;
                let output = source
                    .chars()
                    .skip(start - 1)
                    .take(length.unwrap_or(usize::MAX))
                    .collect::<String>();
                Ok(Cell::Text(output.into_bytes()))
            }
            Cell::Null => Ok(Cell::Null),
            _ => Err(ExecutionError::Invalid(
                "SUBSTRING requires a String value".into(),
            )),
        },
        BoundExpr::Case {
            branches,
            else_expr,
        } => {
            for (condition, value) in branches {
                let matched = match eval_row_arithmetic(condition, row)? {
                    Some(Number::Unsigned(value)) => value != 0,
                    Some(Number::Signed(value)) => value != 0,
                    Some(Number::Float(value)) => value != 0.0,
                    None => false,
                };
                if matched {
                    return eval_row_value(value, row);
                }
            }
            match else_expr {
                Some(value) => eval_row_value(value, row),
                None => Ok(Cell::Null),
            }
        }
        _ => Ok(eval_row_arithmetic(expression, row)?.map_or(Cell::Null, Cell::Numeric)),
    }
}

pub(super) fn eval_number(
    expression: &BoundExpr,
    batch: &[TypedColumn],
    row: usize,
) -> Result<Option<Number>, ExecutionError> {
    match expression {
        BoundExpr::TextLiteral(_) | BoundExpr::TextConcat(_) | BoundExpr::TextSubstring { .. } => {
            Err(ExecutionError::Invalid(
                "text scalar expression cannot be used as a numeric expression".into(),
            ))
        }
        BoundExpr::Round { value, precision } => match eval_number(value, batch, row)? {
            Some(value) => round_number(value, *precision).map(Some),
            None => Ok(None),
        },
        BoundExpr::Case {
            branches,
            else_expr,
        } => {
            for (condition, value) in branches {
                let truthy = match eval_number(condition, batch, row)? {
                    Some(Number::Unsigned(value)) => value != 0,
                    Some(Number::Signed(value)) => value != 0,
                    Some(Number::Float(value)) => value != 0.0,
                    None => false,
                };
                if truthy {
                    return eval_number(value, batch, row);
                }
            }
            match else_expr {
                Some(value) => eval_number(value, batch, row),
                None => Ok(None),
            }
        }
        BoundExpr::TextCompare {
            index,
            operator,
            value,
        } => {
            let source = match &batch[*index] {
                TypedColumn::Text(values) => values.get(row),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => values.get(row),
                TypedColumn::NullableText { .. } => None,
                _ => {
                    return Err(ExecutionError::Invalid(
                        "text expression comparison requires a String column".into(),
                    ));
                }
            };
            Ok(source.map(|source| {
                let ordering = source.cmp(value.as_slice());
                let matched = match operator {
                    crate::Operator::Eq => ordering.is_eq(),
                    crate::Operator::Ne => ordering.is_ne(),
                    crate::Operator::Lt => ordering.is_lt(),
                    crate::Operator::Le => ordering.is_le(),
                    crate::Operator::Gt => ordering.is_gt(),
                    crate::Operator::Ge => ordering.is_ge(),
                    _ => unreachable!("text comparisons require relational operators"),
                };
                Number::Unsigned(u128::from(matched))
            }))
        }
        BoundExpr::TextLike {
            index,
            pattern,
            negated,
            case_insensitive,
        } => {
            let source = match &batch[*index] {
                TypedColumn::Text(values) => values.get(row),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => values.get(row),
                TypedColumn::NullableText { .. } => None,
                _ => {
                    return Err(ExecutionError::Invalid(
                        "LIKE expression requires a String column".into(),
                    ));
                }
            };
            Ok(source.map(|source| {
                let result = typed::text_pattern_matches(source, pattern, *case_insensitive);
                Number::Unsigned(u128::from(result != *negated))
            }))
        }
        BoundExpr::IsNull { index, negated } => Ok(Some(Number::Unsigned(u128::from(
            matches!(
                typed::cell(
                    batch.get(*index).ok_or_else(|| {
                        ExecutionError::Invalid(
                            "NULL expression references an unknown column".into(),
                        )
                    })?,
                    row
                ),
                typed::Cell::Null
            ) == !*negated,
        )))),
        BoundExpr::And(left, right) | BoundExpr::Or(left, right) => {
            let as_bool = |value: Number| match value {
                Number::Unsigned(value) => value != 0,
                Number::Signed(value) => value != 0,
                Number::Float(value) => value != 0.0,
            };
            let left_result = eval_number(left, batch, row)?.map(as_bool);
            let is_and = matches!(expression, BoundExpr::And(..));
            if left_result == Some(!is_and) {
                return Ok(Some(Number::Unsigned(u128::from(!is_and))));
            }
            let right_result = eval_number(right, batch, row)?.map(as_bool);
            let result = match (left_result, right_result) {
                (Some(left), Some(right)) => {
                    Some(if is_and { left && right } else { left || right })
                }
                (Some(left), None) => {
                    if left != is_and {
                        Some(left)
                    } else {
                        None
                    }
                }
                (None, Some(right)) => {
                    if right != is_and {
                        Some(right)
                    } else {
                        None
                    }
                }
                (None, None) => None,
            };
            Ok(result.map(|value| Number::Unsigned(u128::from(value))))
        }
        BoundExpr::Column(index) => numeric_argument_at(batch, *index, row),
        BoundExpr::Compare(left, operator, right) => {
            let Some(left) = eval_number(left, batch, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_number(right, batch, row)? else {
                return Ok(None);
            };
            let order = compare_expression_numbers(left, right);
            let matched = match operator {
                crate::Operator::Eq => order.is_some_and(|order| order.is_eq()),
                crate::Operator::Ne => order.is_none_or(|order| order.is_ne()),
                crate::Operator::Lt => order.is_some_and(|order| order.is_lt()),
                crate::Operator::Le => order.is_some_and(|order| order.is_le()),
                crate::Operator::Gt => order.is_some_and(|order| order.is_gt()),
                crate::Operator::Ge => order.is_some_and(|order| order.is_ge()),
                _ => {
                    return Err(ExecutionError::Invalid(
                        "unsupported numeric comparison operator".into(),
                    ));
                }
            };
            Ok(Some(Number::Unsigned(u128::from(matched))))
        }
        BoundExpr::Integer(value) => Ok(Some(Number::Signed(i128::from(*value)))),
        BoundExpr::Float(value) => Ok(Some(Number::Float(*value))),
        BoundExpr::AffineColumn {
            index,
            constant,
            subtract,
        } => match numeric_argument_at(batch, *index, row)? {
            Some(value) => {
                checked_binary(value, Number::Signed(i128::from(*constant)), *subtract).map(Some)
            }
            None => Ok(None),
        },
        BoundExpr::TextLength(index) => match &batch[*index] {
            // Length only needs the decoded byte span. Do not clone payloads.
            TypedColumn::Text(values) => Ok(values
                .get(row)
                .map(|bytes| Number::Unsigned(bytes.len() as u128))),
            TypedColumn::NullableText { values, nulls } => {
                if nulls[row] != 0 {
                    Ok(None)
                } else {
                    Ok(values
                        .get(row)
                        .map(|bytes| Number::Unsigned(bytes.len() as u128)))
                }
            }
            _ => Err(ExecutionError::Invalid(
                "length requires a String column".into(),
            )),
        },
        BoundExpr::Divide(left, right) => {
            let Some(left) = eval_number(left, batch, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_number(right, batch, row)? else {
                return Ok(None);
            };
            let to_float = |value: Number| match value {
                Number::Unsigned(number) => number as f64,
                Number::Signed(number) => number as f64,
                Number::Float(number) => number,
            };
            let divisor = to_float(right);
            if divisor == 0.0 {
                return Err(ExecutionError::Invalid(
                    "division by zero in aggregate expression".into(),
                ));
            }
            let quotient = to_float(left) / divisor;
            if !quotient.is_finite() {
                return Err(ExecutionError::Invalid(
                    "non-finite aggregate division".into(),
                ));
            }
            Ok(Some(Number::Float(quotient)))
        }
        BoundExpr::Modulo(left, right) => {
            let Some(left) = eval_number(left, batch, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_number(right, batch, row)? else {
                return Ok(None);
            };
            checked_remainder(left, right).map(Some)
        }
        BoundExpr::Multiply(left, right) => {
            let Some(left) = eval_number(left, batch, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_number(right, batch, row)? else {
                return Ok(None);
            };
            checked_multiply(left, right).map(Some)
        }
        BoundExpr::Add(left, right) | BoundExpr::Subtract(left, right) => {
            let Some(left) = eval_number(left, batch, row)? else {
                return Ok(None);
            };
            let Some(right) = eval_number(right, batch, row)? else {
                return Ok(None);
            };
            checked_binary(left, right, matches!(expression, BoundExpr::Subtract(..))).map(Some)
        }
    }
}

fn distinct_argument(
    expression: &BoundExpr,
    batch: &[TypedColumn],
    row: usize,
) -> Result<Option<typed::Key>, ExecutionError> {
    if let BoundExpr::Column(index) = expression {
        let key = match &batch[*index] {
            TypedColumn::Numeric(values) => {
                Some(typed::Key::Numeric(NumberKey::from(number_at(values, row))))
            }
            TypedColumn::Text(values) => values
                .get(row)
                .map(|value| typed::Key::Text(value.to_vec())),
            TypedColumn::Uuid(values) => Some(typed::Key::UuidBytes(*values[row].as_bytes())),
            TypedColumn::NullableNumeric { values, nulls } => (nulls[row] == 0)
                .then(|| typed::Key::Numeric(NumberKey::from(number_at(values, row)))),
            TypedColumn::NullableText { values, nulls } => {
                if nulls[row] != 0 {
                    None
                } else {
                    values
                        .get(row)
                        .map(|value| typed::Key::Text(value.to_vec()))
                }
            }
        };
        return Ok(key);
    }
    Ok(eval_number(expression, batch, row)?.map(|value| Cell::Numeric(value).key()))
}

fn count_argument(
    argument: Option<&BoundExpr>,
    batch: &[TypedColumn],
    row: usize,
) -> Result<bool, ExecutionError> {
    match argument {
        None => Ok(true),
        Some(BoundExpr::Column(index)) => Ok(!typed::is_null_at(&batch[*index], row)),
        Some(expression) => Ok(eval_number(expression, batch, row)?.is_some()),
    }
}

/// Compute independent exact multi-aggregate partials over physical parts,
/// then merge the checked integer SUM and COUNT states. Other aggregate types
/// retain serial ordering to avoid changing floating-point semantics.
fn parallel_exact_multi_aggregates(
    part: &Path,
    names: &[String],
    items: &[AggregateItem],
    arguments: &[Option<BoundExpr>],
    conditions: &[Vec<typed::Condition>],
    no_filters: bool,
) -> Result<Option<Vec<State>>, ExecutionError> {
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = super::query_workers(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    // Parts can differ by orders of magnitude in row count. Claim them
    // dynamically to avoid a worker being stranded with one oversized part.
    // The partial COUNT/SUM states merge in worker order for reproducibility.
    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let chunks = std::thread::scope(|scope| -> Result<_, ExecutionError> {
        let cursor = &cursor;
        let paths = &parts;
        let handles = (0..workers)
            .map(|_| {
                std::thread::Builder::new()
                    .spawn_scoped(scope, move || -> Result<Vec<State>, ExecutionError> {
                        let mut states = items
                            .iter()
                            .map(|item| match item.function {
                                Some(function) => State::Numeric(AggregateState::new(function)),
                                None => State::Count(0),
                            })
                            .collect::<Vec<_>>();
                        while let Some(path) =
                            paths.get(cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
                        {
                            let mut reader = rusthouse_storage::TypedPartReader::open(path, names)?;
                            while let Some(batch) = reader.next_batch()? {
                                for row in 0..batch[0].len() {
                                    if !no_filters && !typed::passes(&batch, row, conditions) {
                                        continue;
                                    }
                                    for (state, argument) in states.iter_mut().zip(arguments) {
                                        match state {
                                            State::Count(value) => {
                                                if count_argument(argument.as_ref(), &batch, row)? {
                                                    *value =
                                                        value.checked_add(1).ok_or_else(|| {
                                                            ExecutionError::Invalid(
                                                                "COUNT overflow".into(),
                                                            )
                                                        })?;
                                                }
                                            }
                                            State::Numeric(value) => {
                                                let expr = argument.as_ref().ok_or_else(|| {
                                                    ExecutionError::Invalid(
                                                        "aggregate argument missing".into(),
                                                    )
                                                })?;
                                                if let Some(next) = eval_number(expr, &batch, row)?
                                                {
                                                    value.update(next)?;
                                                }
                                            }
                                            State::Distinct(_) => {
                                                unreachable!("only exact SUM and COUNT selected")
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(states)
                    })
                    .map_err(|error| {
                        ExecutionError::Invalid(format!("cannot start aggregate worker: {error}"))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(ExecutionError::Invalid(
                        "parallel multi-aggregate worker panicked".into(),
                    ))
                })
            })
            .collect::<Vec<_>>())
    })?;
    let mut totals = items
        .iter()
        .map(|item| match item.function {
            Some(function) => State::Numeric(AggregateState::new(function)),
            None => State::Count(0),
        })
        .collect::<Vec<_>>();
    for chunk in chunks {
        for (total, partial) in totals.iter_mut().zip(chunk?) {
            match (total, partial) {
                (State::Count(total), State::Count(partial)) => {
                    *total = total
                        .checked_add(partial)
                        .ok_or_else(|| ExecutionError::Invalid("COUNT overflow".into()))?;
                }
                (State::Numeric(total), State::Numeric(partial)) => {
                    if let Some(value) = partial.value {
                        total.update(value)?;
                    }
                }
                _ => {
                    return Err(ExecutionError::Invalid(
                        "invalid parallel multi-aggregate state".into(),
                    ));
                }
            }
        }
    }
    Ok(Some(totals))
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    items: &[AggregateItem],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    let mut names = Vec::new();
    for item in items {
        if let Some(expression) = &item.argument {
            expression.referenced_columns(&mut names);
        }
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let name = predicate.column();
        if !names.iter().any(|column| column == name) {
            names.push(name.to_owned());
        }
        if let Some(other) = predicate.other_column() {
            if !names.iter().any(|column| column == other) {
                names.push(other.to_owned());
            }
        }
    }
    if names.is_empty() {
        names.push(
            metadata
                .columns
                .first()
                .ok_or_else(|| ExecutionError::Invalid("table has no columns".into()))?
                .name
                .clone(),
        );
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    for name in &names {
        if !types.contains_key(name) {
            return Err(ExecutionError::Invalid(format!("unknown column: {name}")));
        }
    }
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    writeln!(
        output,
        "{}",
        items
            .iter()
            .map(AggregateItem::header)
            .collect::<Vec<_>>()
            .join("\t")
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }
    // Non-nullable COUNT columns have exactly the snapshot's validated row
    // count. Answer any number of these metrics from metadata, without
    // opening or decompressing a single column stream.
    let metadata_counts = query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && items.iter().all(|item| {
            item.function.is_none()
                && !item.distinct
                && item.argument.as_ref().is_none_or(|expression| {
                    matches!(expression, ScalarExpr::Column(column)
                        if types.get(column).is_some_and(|kind|
                            !kind.starts_with("Nullable(")))
                })
        });
    if metadata_counts {
        if query.offset.unwrap_or(0) == 0 {
            for (index, _) in items.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\t")?;
                }
                write!(output, "{}", metadata.rows)?;
            }
            output.write_all(b"\n")?;
        }
        return Ok(());
    }
    // Mixed nullable COUNTs only need their compact null maps. A full
    // typed value scan decodes every numeric or String payload needlessly.
    // De-duplicate repeated nullable columns across up to 128 metrics.
    let nullmap_counts = query.predicates.is_empty()
        && query.or_predicates.is_empty()
        && items.iter().all(|item| {
            item.function.is_none()
                && !item.distinct
                && item.argument.as_ref().is_none_or(|expression| {
                    matches!(expression, ScalarExpr::Column(column)
                        if types.contains_key(column))
                })
        });
    if nullmap_counts {
        let mut cached = HashMap::<String, u64>::new();
        for item in items {
            let Some(ScalarExpr::Column(column)) = &item.argument else {
                continue;
            };
            if !types[column].starts_with("Nullable(") || cached.contains_key(column) {
                continue;
            }
            let mut reader = TypedQueryReader::open_with_metadata(
                part,
                std::slice::from_ref(column),
                &metadata,
            )?;
            let mut nonnull = 0u64;
            while let Some(nulls) = reader.next_null_batch()? {
                let count = nulls.iter().filter(|&&value| value == 0).count() as u64;
                nonnull = nonnull
                    .checked_add(count)
                    .ok_or_else(|| ExecutionError::Invalid("COUNT overflow".into()))?;
            }
            cached.insert(column.clone(), nonnull);
        }
        if query.offset.unwrap_or(0) == 0 {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.write_all(b"\t")?;
                }
                let count = match &item.argument {
                    Some(ScalarExpr::Column(column)) if types[column].starts_with("Nullable(") => {
                        cached[column]
                    }
                    _ => metadata.rows,
                };
                write!(output, "{count}")?;
            }
            output.write_all(b"\n")?;
        }
        return Ok(());
    }
    let mut states = items
        .iter()
        .map(|item| match item.function {
            Some(function) => State::Numeric(AggregateState::new(function)),
            None if item.distinct => {
                let bitmap32 = match item.argument.as_ref() {
                    Some(ScalarExpr::Column(column))
                        if types.get(column).is_some_and(|kind| {
                            matches!(typed::base_type(kind), "UInt32" | "Int32")
                        }) =>
                    {
                        Some(BitmapDistinct32::new())
                    }
                    _ => None,
                };
                State::Distinct(DistinctValues {
                    bitmap32,
                    ..DistinctValues::default()
                })
            }
            None => State::Count(0),
        })
        .collect::<Vec<_>>();
    let arguments = items
        .iter()
        .map(|item| {
            item.argument
                .as_ref()
                .map(|expression| BoundExpr::bind(expression, &indices))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let parallel_safe =
        items.iter().all(
            |item| match (&item.function, &item.argument, item.distinct) {
                (None, _, false) => true,
                (
                    Some(
                        crate::AggregateFunction::Sum
                        | crate::AggregateFunction::Min
                        | crate::AggregateFunction::Max,
                    ),
                    Some(crate::ScalarExpr::Column(column)),
                    false,
                ) => types.get(column).is_some_and(|kind| {
                    matches!(
                        typed::base_type(kind),
                        "UInt8"
                            | "UInt16"
                            | "UInt32"
                            | "UInt64"
                            | "UInt128"
                            | "Int8"
                            | "Int16"
                            | "Int32"
                            | "Int64"
                            | "Int128"
                    )
                }),
                _ => false,
            },
        );
    // Several affine SUMs over one narrow integer column share one scan and
    // differ only by a constant multiplied by the non-NULL row count.
    let affine = if no_filters {
        affine_sum_states(part, &metadata, &names, &types, items, &arguments)?
    } else {
        None
    };
    let parallel = if affine.is_some() {
        affine
    } else if parallel_safe {
        parallel_exact_multi_aggregates(part, &names, items, &arguments, &conditions, no_filters)?
    } else {
        None
    };
    let scan_serially = parallel.is_none();
    if let Some(partial) = parallel {
        states = partial;
    }
    let mut reader = if scan_serially {
        Some(TypedQueryReader::open_with_metadata(
            part, &names, &metadata,
        )?)
    } else {
        None
    };
    while let Some(batch) = reader
        .as_mut()
        .map_or(Ok(None), TypedQueryReader::next_batch)?
    {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            for (state, argument) in states.iter_mut().zip(&arguments) {
                match state {
                    State::Count(value) => {
                        if count_argument(argument.as_ref(), &batch, row)? {
                            *value = value
                                .checked_add(1)
                                .ok_or_else(|| ExecutionError::Invalid("COUNT overflow".into()))?;
                        }
                    }
                    State::Distinct(seen) => {
                        let expression = argument.as_ref().ok_or_else(|| {
                            ExecutionError::Invalid("distinct argument missing".into())
                        })?;
                        if let BoundExpr::Column(index) = expression {
                            if seen.insert_column(&batch[*index], row)? {
                                continue;
                            }
                        }
                        if let Some(key) = distinct_argument(expression, &batch, row)? {
                            seen.insert_key(key)?;
                        }
                    }
                    State::Numeric(state) => {
                        let expression = argument.as_ref().ok_or_else(|| {
                            ExecutionError::Invalid("aggregate argument missing".into())
                        })?;
                        if let Some(value) = eval_number(expression, &batch, row)? {
                            state.update(value)?;
                        }
                    }
                }
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        for (index, state) in states.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\t")?;
            }
            match state {
                State::Count(count) => write!(output, "{count}")?,
                State::Distinct(seen) => write!(output, "{}", seen.len())?,
                State::Numeric(state) => {
                    let mut value = state.render();
                    if matches!(
                        items[index].function,
                        Some(crate::AggregateFunction::Min | crate::AggregateFunction::Max)
                    ) {
                        if let Some(ScalarExpr::Column(column)) = &items[index].argument {
                            if let Some(data_type) = types.get(column) {
                                value = typed::format_temporal_aggregate(value, data_type);
                            }
                        }
                    }
                    write!(output, "{value}")?;
                }
            }
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

/// Compute exact arithmetic-series metrics in O(number of aggregates) instead
/// of iterating a billion generated rows. Predicated or DISTINCT inputs keep the
/// existing streaming path.
/// Only these generated metrics have a constant-time arithmetic result.
pub(super) fn supports_generated_series_metrics(items: &[AggregateItem]) -> bool {
    items.iter().all(|item| {
        !item.distinct
            && item.argument.as_ref().is_none_or(
                |expression| matches!(expression, ScalarExpr::Column(name) if name == "number"),
            )
            && matches!(
                item.function,
                None | Some(
                    crate::AggregateFunction::Count
                        | crate::AggregateFunction::Sum
                        | crate::AggregateFunction::Min
                        | crate::AggregateFunction::Max
                        | crate::AggregateFunction::Avg
                )
            )
    })
}

fn generated_series_metrics(
    start: u64,
    rows: u64,
    step: u64,
    items: &[AggregateItem],
) -> Option<Result<Vec<String>, ExecutionError>> {
    if !supports_generated_series_metrics(items) {
        return None;
    }
    let first = u128::from(start);
    let n = u128::from(rows);
    let last = if n == 0 {
        first
    } else {
        first + (n - 1) * u128::from(step)
    };
    let sum = if n & 1 == 0 {
        (n / 2) * (first + last)
    } else {
        n * ((first + last) / 2)
    };
    let results = items
        .iter()
        .map(|item| {
            Ok(match item.function {
                None | Some(crate::AggregateFunction::Count) => rows.to_string(),
                Some(crate::AggregateFunction::Sum) if n > 0 => sum.to_string(),
                Some(crate::AggregateFunction::Min) if n > 0 => first.to_string(),
                Some(crate::AggregateFunction::Max) if n > 0 => last.to_string(),
                Some(crate::AggregateFunction::Avg) if n > 0 => {
                    ((first as f64 + last as f64) / 2.0).to_string()
                }
                Some(
                    crate::AggregateFunction::Sum
                    | crate::AggregateFunction::Min
                    | crate::AggregateFunction::Max
                    | crate::AggregateFunction::Avg,
                ) => "NULL".into(),
                _ => unreachable!("fast path excludes other aggregate functions"),
            })
        })
        .collect::<Result<Vec<_>, ExecutionError>>();
    Some(results)
}

/// Evaluate bounded generated-number aggregate projections without a stored part.
pub(super) fn execute_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    items: &[AggregateItem],
    output: &mut W,
) -> Result<(), ExecutionError> {
    for item in items {
        if let Some(argument) = &item.argument {
            if !matches!(argument, ScalarExpr::Column(name) if name == "number") {
                return Err(ExecutionError::Invalid(
                    "generated aggregates support only the number column".into(),
                ));
            }
        } else if item.function.is_some() || item.distinct {
            return Err(ExecutionError::Invalid(
                "generated numeric aggregates require the number column".into(),
            ));
        }
    }
    let comparisons = super::compile_numbers_predicates(query)?;
    writeln!(
        output,
        "{}",
        items
            .iter()
            .map(AggregateItem::header)
            .collect::<Vec<_>>()
            .join("\t")
    )?;
    if query.limit == Some(0) {
        return Ok(());
    }
    // Monotonic predicates select an exact contiguous arithmetic interval.
    // Reuse the formula path for the selected range instead of scanning every
    // generated row and updating each metric separately.
    let interval = if query.predicates.is_empty() && query.or_predicates.is_empty() {
        Some((0, rows))
    } else {
        super::monotonic_range_indices(&comparisons, start, rows, step)
    };
    if let Some((lower, upper)) = interval {
        let first = u128::from(start) + u128::from(lower) * u128::from(step);
        if let Ok(first) = u64::try_from(first) {
            if let Some(results) = generated_series_metrics(first, upper - lower, step, items) {
                if query.offset.unwrap_or(0) == 0 {
                    writeln!(output, "{}", results?.join("\t"))?;
                }
                return Ok(());
            }
        }
    }
    let mut states = items
        .iter()
        .map(|item| match item.function {
            Some(function) => State::Numeric(AggregateState::new(function)),
            None if item.distinct => State::Distinct(DistinctValues::default()),
            None => State::Count(0),
        })
        .collect::<Vec<_>>();
    // Equality and IN seeds produce at most 4096 exact generated positions.
    // Evaluate only those positions, rather than scanning a billion-row source.
    let sparse = if query.predicates.is_empty() && query.or_predicates.is_empty() {
        None
    } else {
        super::sparse_generated_positions(&comparisons, start, rows, step)
    };
    let mut consume = |index: u64| -> Result<(), ExecutionError> {
        let value = Number::Unsigned(u128::from(start) + u128::from(index) * u128::from(step));
        for state in &mut states {
            match state {
                State::Count(count) => {
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| ExecutionError::Invalid("COUNT overflow".into()))?;
                }
                State::Distinct(seen) => {
                    seen.insert_key(typed::Key::Numeric(NumberKey::from(value)))?;
                }
                State::Numeric(state) => state.update(value)?,
            }
        }
        Ok(())
    };
    if let Some(indices) = sparse {
        for index in indices {
            consume(index)?;
        }
    } else {
        for index in 0..rows {
            let value = Number::Unsigned(u128::from(start) + u128::from(index) * u128::from(step));
            if super::number_matches_all(value, &comparisons) {
                consume(index)?;
            }
        }
    }
    if query.offset.unwrap_or(0) == 0 {
        for (index, state) in states.iter().enumerate() {
            if index > 0 {
                output.write_all(b"\t")?;
            }
            match state {
                State::Count(count) => write!(output, "{count}")?,
                State::Distinct(seen) => write!(output, "{}", seen.len())?,
                State::Numeric(state) => write!(output, "{}", state.render())?,
            }
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod affine_tests {
    use super::*;

    #[test]
    fn binds_column_arithmetic_without_recursive_dispatch() {
        let indices = HashMap::from([("width".to_owned(), 0_usize)]);
        let column = || ScalarExpr::Column("width".into());
        let literal = || ScalarExpr::Integer(3);
        let add = ScalarExpr::Add(Box::new(column()), Box::new(literal()));
        let sub = ScalarExpr::Subtract(Box::new(column()), Box::new(literal()));

        assert!(matches!(
            BoundExpr::bind(&add, &indices).unwrap(),
            BoundExpr::AffineColumn {
                index: 0,
                constant: 3,
                subtract: false,
            }
        ));
        assert!(matches!(
            BoundExpr::bind(&sub, &indices).unwrap(),
            BoundExpr::AffineColumn {
                index: 0,
                constant: 3,
                subtract: true,
            }
        ));

        // Reversed addition must keep the generic path until it is proven safe.
        let reversed = ScalarExpr::Add(Box::new(literal()), Box::new(column()));
        assert!(matches!(
            BoundExpr::bind(&reversed, &indices).unwrap(),
            BoundExpr::Add(_, _)
        ));
    }
}

#[cfg(test)]
mod text_projection_tests {
    use super::*;

    #[test]
    fn concatenates_coalesced_text_from_joined_rows() {
        let expression = crate::expression::parse_scalar_expression(
            "concat(concat(coalesce(last_name, ''), ', '), coalesce(first_name, ''))",
        )
        .unwrap();
        let indices = HashMap::from([
            ("last_name".to_owned(), 0_usize),
            ("first_name".to_owned(), 1_usize),
        ]);
        let bound = BoundExpr::bind(&expression, &indices).unwrap();
        let row = vec![Cell::Text(b"Vu".to_vec()), Cell::Null];

        assert!(matches!(
            eval_row_value(&bound, &row).unwrap(),
            Cell::Text(value) if value == b"Vu, "
        ));
    }

    #[test]
    fn concat_skips_null_arguments() {
        let expression = crate::expression::parse_scalar_expression("concat(name, '!')").unwrap();
        let bound =
            BoundExpr::bind(&expression, &HashMap::from([("name".to_owned(), 0_usize)])).unwrap();

        assert!(matches!(
            eval_row_value(&bound, &[Cell::Null]).unwrap(),
            Cell::Text(value) if value == b"!"
        ));
    }

    #[test]
    fn concat_returns_empty_text_when_all_arguments_are_null() {
        let expression = crate::expression::parse_scalar_expression("concat(left, right)").unwrap();
        let bound = BoundExpr::bind(
            &expression,
            &HashMap::from([("left".to_owned(), 0_usize), ("right".to_owned(), 1_usize)]),
        )
        .unwrap();

        assert!(matches!(
            eval_row_value(&bound, &[Cell::Null, Cell::Null]).unwrap(),
            Cell::Text(value) if value.is_empty()
        ));
    }

    #[test]
    fn substring_counts_utf8_characters_and_handles_empty_ranges() {
        let expression =
            crate::expression::parse_scalar_expression("substring('A界BC', 2, 2)").unwrap();
        let bound = BoundExpr::bind(&expression, &HashMap::new()).unwrap();

        assert!(matches!(
            eval_row_value(&bound, &[]).unwrap(),
            Cell::Text(value) if value == "界B".as_bytes()
        ));
    }

    #[test]
    fn round_handles_signed_values_nulls_and_negative_precision() {
        let expression = crate::expression::parse_scalar_expression("round(value, 2)").unwrap();
        let bound =
            BoundExpr::bind(&expression, &HashMap::from([("value".to_owned(), 0)])).unwrap();

        assert!(matches!(
            eval_row_value(&bound, &[Cell::Numeric(Number::Float(1.235))]).unwrap(),
            Cell::Numeric(Number::Float(value)) if (value - 1.24).abs() < 1e-12
        ));
        assert!(matches!(
            eval_row_value(&bound, &[Cell::Numeric(Number::Float(-1.235))]).unwrap(),
            Cell::Numeric(Number::Float(value)) if (value + 1.24).abs() < 1e-12
        ));
        assert!(matches!(
            eval_row_value(&bound, &[Cell::Null]).unwrap(),
            Cell::Null
        ));

        let expression = crate::expression::parse_scalar_expression("round(value, -2)").unwrap();
        let bound =
            BoundExpr::bind(&expression, &HashMap::from([("value".to_owned(), 0)])).unwrap();
        assert!(matches!(
            eval_row_value(&bound, &[Cell::Numeric(Number::Signed(1250))]).unwrap(),
            Cell::Numeric(Number::Signed(1300))
        ));
    }
}

#[cfg(test)]
mod distinct_key_tests {
    use super::*;
    use rusthouse_storage::read_legacy_strings;

    #[test]
    fn distinct_state_uses_borrowed_text_and_fixed_width_uuid_keys() {
        let mut input = b"\x03abc\x03abc\x03xyz".as_slice();
        let values = read_legacy_strings(&mut input, 3).unwrap();
        let column = TypedColumn::Text(values);
        let mut distinct = DistinctValues::default();
        for row in 0..3 {
            assert!(distinct.insert_column(&column, row).unwrap());
        }
        assert_eq!(distinct.len(), 2);
        assert!(distinct.other.is_empty());
        assert_eq!(distinct.text.len(), 2);

        let nullable = TypedColumn::NullableNumeric {
            values: rusthouse_storage::NumericColumn::UInt64(vec![1]),
            nulls: vec![1],
        };
        assert!(!distinct.insert_column(&nullable, 0).unwrap());
    }

    #[test]
    fn distinct_argument_preserves_nullable_string_values() {
        let mut stream = b"\x03abc\x03abc".as_slice();
        let strings = read_legacy_strings(&mut stream, 2).unwrap();
        let batch = vec![TypedColumn::NullableText {
            values: strings,
            nulls: vec![0, 1],
        }];
        let expression = BoundExpr::Column(0);
        assert!(matches!(
            distinct_argument(&expression, &batch, 0).unwrap(),
            Some(typed::Key::Text(value)) if value == b"abc"
        ));
        assert!(distinct_argument(&expression, &batch, 1).unwrap().is_none());
    }
}

#[cfg(test)]
mod tpch_decimal_literal_tests {
    use super::*;

    #[test]
    fn binds_and_evaluates_decimal_constant_once() {
        let bound =
            BoundExpr::bind(&ScalarExpr::Decimal("0.05".to_owned()), &HashMap::new()).unwrap();
        let value = eval_number(&bound, &[], 0).unwrap();
        assert!(matches!(value, Some(Number::Float(f)) if (f - 0.05).abs() < 1e-15));
    }
}

#[cfg(test)]
mod direct_numeric_argument_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn evaluates_numeric_columns_and_preserves_nulls() {
        let batch = vec![
            TypedColumn::Numeric(NumericColumn::UInt64(vec![100, 200])),
            TypedColumn::NullableNumeric {
                values: NumericColumn::Int64(vec![-5, 7]),
                nulls: vec![0, 1],
            },
        ];
        assert!(matches!(
            eval_number(&BoundExpr::Column(0), &batch, 1).unwrap(),
            Some(Number::Unsigned(200))
        ));
        assert!(matches!(
            eval_number(&BoundExpr::Column(1), &batch, 0).unwrap(),
            Some(Number::Signed(-5))
        ));
        assert!(
            eval_number(&BoundExpr::Column(1), &batch, 1)
                .unwrap()
                .is_none()
        );
        let affine = BoundExpr::AffineColumn {
            index: 0,
            constant: 5,
            subtract: false,
        };
        assert!(matches!(
            eval_number(&affine, &batch, 0).unwrap(),
            Some(Number::Unsigned(105)) | Some(Number::Signed(105))
        ));
    }
}

#[cfg(test)]
mod affine_sum_fast_path_tests {
    use super::*;

    fn sum_item(column: &str, constant: i64) -> AggregateItem {
        AggregateItem {
            function: Some(crate::AggregateFunction::Sum),
            argument: Some(ScalarExpr::Add(
                Box::new(ScalarExpr::Column(column.to_owned())),
                Box::new(ScalarExpr::Integer(constant)),
            )),
            distinct: false,
            alias: None,
        }
    }

    fn affine(index: usize, constant: i64) -> Option<BoundExpr> {
        Some(BoundExpr::AffineColumn {
            index,
            constant,
            subtract: false,
        })
    }

    fn same_number(left: Option<Number>, right: Option<Number>) -> bool {
        match (left, right) {
            (None, None) => true,
            (Some(Number::Unsigned(left)), Some(Number::Unsigned(right))) => left == right,
            (Some(Number::Signed(left)), Some(Number::Signed(right))) => left == right,
            (Some(Number::Float(left)), Some(Number::Float(right))) => {
                left.to_bits() == right.to_bits()
            }
            _ => false,
        }
    }

    #[test]
    fn affine_sum_formula_matches_row_updates_for_unsigned_columns() {
        let batch = [TypedColumn::Numeric(NumericColumn::UInt16(vec![
            0, 1, 17, 65_535,
        ]))];
        let mut base_sum = None;
        let mut non_null_rows = 0;
        accumulate_affine_base(&batch[0], &mut base_sum, &mut non_null_rows).unwrap();

        for constant in [0, 1, 90] {
            let expression = BoundExpr::AffineColumn {
                index: 0,
                constant,
                subtract: false,
            };
            let mut expected = AggregateState::new(crate::AggregateFunction::Sum);
            for row in 0..batch[0].len() {
                expected
                    .update_sum(eval_number(&expression, &batch, row).unwrap().unwrap())
                    .unwrap();
            }
            let actual = affine_sum_values(base_sum, non_null_rows, &[constant]).unwrap()[0];
            assert!(same_number(actual, expected.value));
        }
    }

    #[test]
    fn affine_sum_formula_skips_nulls_and_preserves_all_null_sum() {
        let nullable = TypedColumn::NullableNumeric {
            values: NumericColumn::Int16(vec![2, 99, -4]),
            nulls: vec![0, 1, 0],
        };
        let mut base_sum = None;
        let mut non_null_rows = 0;
        accumulate_affine_base(&nullable, &mut base_sum, &mut non_null_rows).unwrap();
        assert!(matches!(
            affine_sum_values(base_sum, non_null_rows, &[0, 5])
                .unwrap()
                .as_slice(),
            [Some(Number::Signed(-2)), Some(Number::Signed(8))]
        ));

        let all_null = TypedColumn::NullableNumeric {
            values: NumericColumn::UInt16(vec![1, 2]),
            nulls: vec![1, 1],
        };
        let mut base_sum = None;
        let mut non_null_rows = 0;
        accumulate_affine_base(&all_null, &mut base_sum, &mut non_null_rows).unwrap();
        assert!(
            affine_sum_values(base_sum, non_null_rows, &[1, 90])
                .unwrap()
                .iter()
                .all(Option::is_none)
        );
    }

    #[test]
    fn affine_sum_formula_keeps_checked_overflow_behavior() {
        assert!(affine_sum_values(Some(Number::Unsigned(u128::MAX)), 1, &[1]).is_err());
        assert!(affine_sum_values(Some(Number::Signed(i128::MAX)), 1, &[1]).is_err());
    }

    #[test]
    fn affine_sum_plan_requires_one_nonnegative_additive_integer_column() {
        assert_eq!(
            affine_sum_plan(
                &[sum_item("width", 1), sum_item("width", 2)],
                &[affine(0, 1), affine(0, 2)],
            ),
            Some((0, vec![1, 2]))
        );
        assert_eq!(
            affine_sum_plan(
                &[
                    AggregateItem {
                        function: Some(crate::AggregateFunction::Sum),
                        argument: Some(ScalarExpr::Column("width".into())),
                        distinct: false,
                        alias: None,
                    },
                    sum_item("width", 2),
                ],
                &[Some(BoundExpr::Column(0)), affine(0, 2)],
            ),
            Some((0, vec![0, 2]))
        );
        assert!(
            affine_sum_plan(
                &[sum_item("width", 1), sum_item("height", 2)],
                &[affine(0, 1), affine(1, 2)],
            )
            .is_none()
        );
        assert!(affine_sum_plan(&[sum_item("width", -1)], &[affine(0, -1)]).is_none());
        assert!(
            affine_sum_plan(
                &[AggregateItem {
                    function: Some(crate::AggregateFunction::Avg),
                    ..sum_item("width", 1)
                }],
                &[affine(0, 1)],
            )
            .is_none()
        );
        assert!(
            affine_sum_plan(
                &[sum_item("width", 1)],
                &[Some(BoundExpr::AffineColumn {
                    index: 0,
                    constant: 1,
                    subtract: true,
                })],
            )
            .is_none()
        );
    }

    #[test]
    fn affine_sum_range_guard_accepts_nullable_narrow_types_and_rejects_wide_types() {
        assert!(affine_sum_range_is_safe("Nullable(UInt16)", 1_000_000, 90));
        assert!(affine_sum_range_is_safe("Int32", 1_000, 5));
        assert!(!affine_sum_range_is_safe("UInt64", 1_000_000, 90));
        assert!(!affine_sum_range_is_safe("UInt16", u64::MAX, i64::MAX));
    }
}

#[cfg(test)]
mod mixed_integer_expression_comparison_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn unsigned_columns_compare_numerically_against_signed_literals() {
        let batch = [TypedColumn::Numeric(NumericColumn::UInt64(vec![0, 2, 3]))];
        let expression = BoundExpr::Compare(
            Box::new(BoundExpr::Column(0)),
            crate::Operator::Gt,
            Box::new(BoundExpr::Integer(2)),
        );
        for (row, expected) in [(0, 0u128), (1, 0), (2, 1)] {
            assert!(matches!(eval_number(&expression, &batch, row).unwrap(),
                Some(Number::Unsigned(value)) if value == expected));
        }
        assert_eq!(
            compare_expression_numbers(Number::Unsigned(u128::MAX), Number::Signed(-1)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            compare_expression_numbers(Number::Signed(i128::MIN), Number::Unsigned(0)),
            Some(std::cmp::Ordering::Less)
        );
    }
}

#[cfg(test)]
mod ieee_comparison_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn compares_float_zero_normally_and_nan_as_unordered() {
        assert_eq!(
            compare_expression_numbers(Number::Float(-0.0), Number::Float(0.0)),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            compare_expression_numbers(Number::Float(f64::NAN), Number::Float(f64::NAN)),
            None
        );
        let batch = [
            TypedColumn::Numeric(NumericColumn::Float64(vec![-0.0, f64::NAN])),
            TypedColumn::Numeric(NumericColumn::Float64(vec![0.0, f64::NAN])),
        ];
        for (operator, row, expected) in [
            (crate::Operator::Eq, 0, 1),
            (crate::Operator::Ne, 0, 0),
            (crate::Operator::Eq, 1, 0),
            (crate::Operator::Ne, 1, 1),
            (crate::Operator::Lt, 1, 0),
            (crate::Operator::Ge, 1, 0),
        ] {
            let expression = BoundExpr::Compare(
                Box::new(BoundExpr::Column(0)),
                operator,
                Box::new(BoundExpr::Column(1)),
            );
            assert!(matches!(eval_number(&expression, &batch, row).unwrap(),
                Some(Number::Unsigned(actual)) if actual == expected));
        }
    }
}

#[cfg(test)]
mod exact_mixed_float_integer_comparison_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;
    use std::cmp::Ordering;

    #[test]
    fn integer_float_comparisons_keep_u128_precision() {
        let border = 9_007_199_254_740_992u128;
        assert_eq!(
            compare_unsigned_float(border + 1, border as f64),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_unsigned_float(u128::MAX, 2.0_f64.powi(128)),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_unsigned_float(u128::MAX, f64::INFINITY),
            Some(Ordering::Less)
        );
        assert_eq!(compare_unsigned_float(0, -0.0), Some(Ordering::Equal));
        assert_eq!(compare_signed_float(-1, -1.5), Some(Ordering::Greater));
        assert_eq!(
            compare_signed_float(i128::MIN, -2.0_f64.powi(127)),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_signed_float(-1, f64::NEG_INFINITY),
            Some(Ordering::Greater)
        );
        assert_eq!(compare_unsigned_float(1, f64::NAN), None);
        assert_eq!(
            compare_expression_numbers(Number::Float(-1.5), Number::Signed(-1)),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn float_columns_compare_against_signed_constant_literals() {
        let batch = [TypedColumn::Numeric(NumericColumn::Float64(vec![
            -1.0,
            3.0,
            4.5,
            f64::NAN,
        ]))];
        let comparison = BoundExpr::Compare(
            Box::new(BoundExpr::Column(0)),
            crate::Operator::Gt,
            Box::new(BoundExpr::Integer(3)),
        );
        for (index, expected) in [0u128, 0, 1, 0].into_iter().enumerate() {
            assert!(matches!(eval_number(&comparison, &batch, index).unwrap(),
                Some(Number::Unsigned(actual)) if actual == expected));
        }
    }
}

#[cfg(test)]
mod multi_aggregate_distinct_text_budget_tests {
    use super::*;

    #[test]
    fn duplicate_text_keys_do_not_consume_additional_memory() {
        let mut distinct = DistinctValues::default();
        distinct.insert_text(b"long-segment-key").unwrap();
        let first = distinct.retained_text_bytes;
        distinct.insert_text(b"long-segment-key").unwrap();
        assert_eq!(first, distinct.retained_text_bytes);
        assert_eq!(distinct.len(), 1);
    }

    #[test]
    fn text_budget_refuses_new_keys_before_mutating_set() {
        let mut distinct = DistinctValues {
            retained_text_bytes: MAX_DISTINCT_TEXT_BYTES - 1,
            ..Default::default()
        };
        assert!(distinct.insert_text(b"new-key").is_err());
        assert_eq!(distinct.retained_text_bytes, MAX_DISTINCT_TEXT_BYTES - 1);
        assert!(distinct.text.is_empty());
    }
}

#[cfg(test)]
mod dynamic_multiagg_workers_tests {
    #[test]
    fn dynamic_cursor_distributes_all_uneven_parts_exactly_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cursor = AtomicUsize::new(0);
        let mut indices = std::thread::scope(|scope| {
            let handles = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        let mut mine = Vec::new();
                        while let index @ 0..=250 = cursor.fetch_add(1, Ordering::Relaxed) {
                            mine.push(index);
                        }
                        mine
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .flat_map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        indices.sort_unstable();
        assert_eq!(indices, (0..=250).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod sparse_generated_multi_metric_tests {
    use crate::{execute, parse_select};

    fn run(sql: &str) -> String {
        let query = parse_select(sql).unwrap();
        let mut output = Vec::new();
        execute(&query, None, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    #[test]
    fn billion_row_sparse_or_predicates_are_deduplicated() {
        assert_eq!(
            run(
                "SELECT sum(number), count(*) FROM numbers(1000000000) WHERE number IN (42, 99) OR number = 42 OR number = 999999998"
            ),
            "sum(number)\tcount(*)\n1000000139\t3\n",
        );
    }

    #[test]
    fn large_sparse_multi_metrics_preserve_exact_min_max_and_count() {
        assert_eq!(
            run(
                "SELECT count(*), min(number), max(number) FROM numbers(1000000000) WHERE number IN (2, 4, 6)"
            ),
            "count(*)\tmin(number)\tmax(number)\n3\t2\t6\n",
        );
    }
}
