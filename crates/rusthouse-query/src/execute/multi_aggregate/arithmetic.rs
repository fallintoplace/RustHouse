use super::*;

pub(super) fn checked_binary(left: Number, right: Number, subtract: bool) -> Result<Number, ExecutionError> {
    let op = |a: i128, b: i128| {
        if subtract {
            a.checked_sub(b)
        } else {
            a.checked_add(b)
        }
    };
    match (left, right) {
        (Number::Float(a), b) => {
            let b = match b {
                Number::Float(b) => b,
                Number::Signed(b) => b as f64,
                Number::Unsigned(b) => b as f64,
            };
            Ok(Number::Float(if subtract { a - b } else { a + b }))
        }
        (a, Number::Float(b)) => {
            let a = match a {
                Number::Float(a) => a,
                Number::Signed(a) => a as f64,
                Number::Unsigned(a) => a as f64,
            };
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
            let to_i128 = |number: Number| -> Result<i128, ExecutionError> {
                match number {
                    Number::Unsigned(number) => i128::try_from(number).map_err(|_| {
                        ExecutionError::Invalid("aggregate expression exceeds Int128".into())
                    }),
                    Number::Signed(number) => Ok(number),
                    Number::Float(_) => unreachable!(),
                }
            };
            op(to_i128(a)?, to_i128(b)?)
                .map(Number::Signed)
                .ok_or_else(|| ExecutionError::Invalid("aggregate expression overflow".into()))
        }
    }
}

pub(super) fn checked_multiply(left: Number, right: Number) -> Result<Number, ExecutionError> {
    let as_float = |number: Number| match number {
        Number::Float(value) => value,
        Number::Signed(value) => value as f64,
        Number::Unsigned(value) => value as f64,
    };
    match (left, right) {
        (Number::Float(_), _) | (_, Number::Float(_)) => {
            Ok(Number::Float(as_float(left) * as_float(right)))
        }
        (Number::Unsigned(a), Number::Unsigned(b)) => {
            a.checked_mul(b).map(Number::Unsigned).ok_or_else(|| {
                ExecutionError::Invalid("unsigned aggregate expression overflow".into())
            })
        }
        (a, b) => {
            let signed = |number: Number| match number {
                Number::Signed(value) => Ok(value),
                Number::Unsigned(value) => i128::try_from(value).map_err(|_| {
                    ExecutionError::Invalid("aggregate expression exceeds Int128".into())
                }),
                Number::Float(_) => unreachable!(),
            };
            signed(a)?
                .checked_mul(signed(b)?)
                .map(Number::Signed)
                .ok_or_else(|| ExecutionError::Invalid("aggregate expression overflow".into()))
        }
    }
}
