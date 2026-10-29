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

/// Checked unary functions for numeric aggregate arguments.
pub(super) fn apply_unary_math(name: &str, value: Number) -> Result<Number, ExecutionError> {
    let result = match name {
        "SIGN" => match value {
            Number::Signed(number) => Number::Signed(number.signum()),
            Number::Unsigned(number) => Number::Unsigned(u128::from(number != 0)),
            Number::Float(number) => {
                if number == 0.0 {
                    Number::Float(0.0)
                } else {
                    Number::Float(number.signum())
                }
            }
        },
        _ => {
            return Err(ExecutionError::Invalid(format!(
                "unsupported aggregate numeric function: {name}"
            )));
        }
    };
    if matches!(result, Number::Float(value) if !value.is_finite()) {
        return Err(ExecutionError::Invalid(
            "non-finite result in aggregate numeric function".into(),
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod aggregate_unary_math_tests {
    use super::*;

    #[test]
    fn sign_preserves_integral_precision_and_nullable_arguments() {
        assert!(matches!(
            apply_unary_math("SIGN", Number::Signed(i128::MIN)),
            Ok(Number::Signed(-1))
        ));
        assert!(matches!(
            apply_unary_math("SIGN", Number::Unsigned(u128::MAX)),
            Ok(Number::Unsigned(1))
        ));
        assert!(matches!(
            apply_unary_math("SIGN", Number::Float(0.0)),
            Ok(Number::Float(value)) if value == 0.0
        ));
        assert!(matches!(
            apply_unary_math("SIGN", Number::Float(-0.0)),
            Ok(Number::Float(value)) if value == 0.0
        ));
        assert!(matches!(
            super::eval_number(
                &BoundExpr::UnaryMath("SIGN".into(), Box::new(BoundExpr::Null)),
                &[],
                0,
            ),
            Ok(None)
        ));
        assert!(crate::parse_scalar_expression("sign(amount)").is_ok());
        assert!(crate::parse_scalar_expression("sign(amount, 2)").is_err());
    }
}
