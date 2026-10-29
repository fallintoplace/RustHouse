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
        "NEGATE" => match value {
            Number::Signed(number) if number == i128::MIN => {
                Number::Unsigned(1_u128 << 127)
            }
            Number::Signed(number) => Number::Signed(-number),
            Number::Unsigned(number) if number == 1_u128 << 127 => {
                Number::Signed(i128::MIN)
            }
            Number::Unsigned(number) => {
                let magnitude = i128::try_from(number).map_err(|_| {
                    ExecutionError::Invalid("aggregate negate exceeds Int128".into())
                })?;
                Number::Signed(-magnitude)
            }
            Number::Float(number) => Number::Float(-number),
        },
        "SQRT" => Number::Float(number_as_float(value).sqrt()),
        "CBRT" => Number::Float(number_as_float(value).cbrt()),
        "FLOOR" => match value {
            Number::Float(number) => Number::Float(number.floor()),
            integer => integer,
        },
        "CEIL" => match value {
            Number::Float(number) => Number::Float(number.ceil()),
            integer => integer,
        },
        "TRUNC" => match value {
            Number::Float(number) => Number::Float(number.trunc()),
            integer => integer,
        },
        "ROUND" => match value {
            Number::Float(number) => Number::Float(number.round_ties_even()),
            integer => integer,
        },
        "EXP" => Number::Float(number_as_float(value).exp()),
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
    #[test]
    fn negate_preserves_signed_boundaries_and_rejects_unrepresentable_values() {
        assert!(matches!(
            apply_unary_math("NEGATE", Number::Signed(i128::MIN)),
            Ok(Number::Unsigned(value)) if value == 1_u128 << 127
        ));
        assert!(matches!(
            apply_unary_math("NEGATE", Number::Unsigned(1_u128 << 127)),
            Ok(Number::Signed(i128::MIN))
        ));
        assert!(apply_unary_math("NEGATE", Number::Unsigned(u128::MAX)).is_err());
        assert!(crate::parse_scalar_expression("negate(amount)").is_ok());
        assert!(crate::parse_scalar_expression("negate(amount, 2)").is_err());
    }

    #[test]
    fn sqrt_supports_positive_values_and_rejects_negative_inputs() {
        assert!(matches!(
            apply_unary_math("SQRT", Number::Unsigned(81)),
            Ok(Number::Float(value)) if value == 9.0
        ));
        assert!(apply_unary_math("SQRT", Number::Signed(-1)).is_err());
        assert!(crate::parse_scalar_expression("sqrt(amount)").is_ok());
        assert!(crate::parse_scalar_expression("sqrt(amount, 2)").is_err());
    }

    #[test]
    fn cbrt_accepts_negative_values_and_preserves_zero() {
        assert!(matches!(
            apply_unary_math("CBRT", Number::Signed(-27)),
            Ok(Number::Float(value)) if (value + 3.0).abs() < 1e-12
        ));
        assert!(matches!(
            apply_unary_math("CBRT", Number::Unsigned(0)),
            Ok(Number::Float(value)) if value == 0.0
        ));
        assert!(crate::parse_scalar_expression("cbrt(amount)").is_ok());
        assert!(crate::parse_scalar_expression("cbrt(amount, 2)").is_err());
    }

    #[test]
    fn floor_rounds_negative_floats_down_without_losing_integer_precision() {
        assert!(matches!(
            apply_unary_math("FLOOR", Number::Float(-1.2)),
            Ok(Number::Float(value)) if value == -2.0
        ));
        assert!(matches!(
            apply_unary_math("FLOOR", Number::Unsigned(u128::MAX)),
            Ok(Number::Unsigned(u128::MAX))
        ));
        assert!(crate::parse_scalar_expression("floor(amount)").is_ok());
        assert!(crate::parse_scalar_expression("floor(amount, 2)").is_err());
    }

    #[test]
    fn ceil_rounds_positive_floats_up_and_preserves_wide_integers() {
        assert!(matches!(
            apply_unary_math("CEIL", Number::Float(1.2)),
            Ok(Number::Float(value)) if value == 2.0
        ));
        assert!(matches!(
            apply_unary_math("CEIL", Number::Signed(i128::MIN)),
            Ok(Number::Signed(i128::MIN))
        ));
        assert!(crate::parse_scalar_expression("ceil(amount)").is_ok());
        assert!(crate::parse_scalar_expression("ceil(amount, 2)").is_err());
    }

    #[test]
    fn trunc_rounds_toward_zero_for_negative_floats() {
        assert!(matches!(
            apply_unary_math("TRUNC", Number::Float(-2.9)),
            Ok(Number::Float(value)) if value == -2.0
        ));
        assert!(matches!(
            apply_unary_math("TRUNC", Number::Unsigned(u128::MAX)),
            Ok(Number::Unsigned(u128::MAX))
        ));
        assert!(crate::parse_scalar_expression("trunc(amount)").is_ok());
        assert!(crate::parse_scalar_expression("trunc(amount, 2)").is_err());
    }

    #[test]
    fn round_uses_ties_to_even_without_rounding_integers() {
        assert!(matches!(
            apply_unary_math("ROUND", Number::Float(2.5)),
            Ok(Number::Float(value)) if value == 2.0
        ));
        assert!(matches!(
            apply_unary_math("ROUND", Number::Float(3.5)),
            Ok(Number::Float(value)) if value == 4.0
        ));
        assert!(matches!(
            apply_unary_math("ROUND", Number::Unsigned(u128::MAX)),
            Ok(Number::Unsigned(u128::MAX))
        ));
        assert!(crate::parse_scalar_expression("round(amount)").is_ok());
        assert!(crate::parse_scalar_expression("round(amount, 2)").is_err());
    }

    #[test]
    fn exp_supports_zero_and_rejects_float_overflow() {
        assert!(matches!(
            apply_unary_math("EXP", Number::Signed(0)),
            Ok(Number::Float(value)) if value == 1.0
        ));
        assert!(apply_unary_math("EXP", Number::Signed(1000)).is_err());
        assert!(crate::parse_scalar_expression("exp(amount)").is_ok());
        assert!(crate::parse_scalar_expression("exp(amount, 2)").is_err());
    }

}
