use super::*;

fn number_as_float(number: Number) -> f64 {
    match number {
        Number::Float(value) => value,
        Number::Signed(value) => value as f64,
        Number::Unsigned(value) => value as f64,
    }
}

fn number_as_i128(number: Number) -> Result<i128, ExecutionError> {
    match number {
        Number::Signed(value) => Ok(value),
        Number::Unsigned(value) => i128::try_from(value)
            .map_err(|_| ExecutionError::Invalid("aggregate expression exceeds Int128".into())),
        Number::Float(_) => unreachable!("floats handled before integer coercion"),
    }
}

pub(super) fn checked_binary(
    left: Number,
    right: Number,
    subtract: bool,
) -> Result<Number, ExecutionError> {
    let op = |a: i128, b: i128| {
        if subtract {
            a.checked_sub(b)
        } else {
            a.checked_add(b)
        }
    };
    match (left, right) {
        (Number::Float(a), b) => {
            let b = number_as_float(b);
            Ok(Number::Float(if subtract { a - b } else { a + b }))
        }
        (a, Number::Float(b)) => {
            let a = number_as_float(a);
            Ok(Number::Float(if subtract { a - b } else { a + b }))
        }
        (Number::Unsigned(a), Number::Unsigned(b)) => {
            let value = if subtract {
                a.checked_sub(b)
            } else {
                a.checked_add(b)
            };
            value.map(Number::Unsigned).ok_or_else(|| {
                ExecutionError::Invalid("unsigned aggregate expression overflow".into())
            })
        }
        (a, b) => {
            op(number_as_i128(a)?, number_as_i128(b)?)
                .map(Number::Signed)
                .ok_or_else(|| ExecutionError::Invalid("aggregate expression overflow".into()))
        }
    }
}

pub(super) fn checked_multiply(left: Number, right: Number) -> Result<Number, ExecutionError> {
    match (left, right) {
        (Number::Float(_), _) | (_, Number::Float(_)) => {
            Ok(Number::Float(number_as_float(left) * number_as_float(right)))
        }
        (Number::Unsigned(a), Number::Unsigned(b)) => {
            a.checked_mul(b).map(Number::Unsigned).ok_or_else(|| {
                ExecutionError::Invalid("unsigned aggregate expression overflow".into())
            })
        }
        (a, b) => {
            number_as_i128(a)?
                .checked_mul(number_as_i128(b)?)
                .map(Number::Signed)
                .ok_or_else(|| ExecutionError::Invalid("aggregate expression overflow".into()))
        }
    }
}
