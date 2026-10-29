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

fn mixed_negative(magnitude: u128) -> Result<Number, ExecutionError> {
    if magnitude <= i128::MAX as u128 {
        return Ok(Number::Signed(-(magnitude as i128)));
    }
    if magnitude == (1_u128 << 127) {
        return Ok(Number::Signed(i128::MIN));
    }
    Err(ExecutionError::Invalid("aggregate expression overflow".into()))
}

fn mixed_positive(magnitude: u128) -> Number {
    match i128::try_from(magnitude) {
        Ok(value) => Number::Signed(value),
        Err(_) => Number::Unsigned(magnitude),
    }
}

/// Add an unsigned value to a signed offset without narrowing UInt128 to
/// Int128. Preserve Signed results in the ordinary Int128 range.
fn unsigned_with_signed(
    unsigned: u128,
    signed: i128,
    subtract: bool,
) -> Result<Number, ExecutionError> {
    let magnitude = signed.unsigned_abs();
    let add = (signed >= 0) != subtract;
    if add {
        unsigned.checked_add(magnitude).map(mixed_positive)
            .ok_or_else(|| ExecutionError::Invalid("aggregate expression overflow".into()))
    } else if unsigned >= magnitude {
        Ok(mixed_positive(unsigned - magnitude))
    } else {
        mixed_negative(magnitude - unsigned)
    }
}

fn signed_minus_unsigned(signed: i128, unsigned: u128) -> Result<Number, ExecutionError> {
    if signed >= 0 {
        if (signed as u128) >= unsigned {
            Ok(Number::Signed(((signed as u128) - unsigned) as i128))
        } else {
            mixed_negative(unsigned - signed as u128)
        }
    } else {
        signed.unsigned_abs().checked_add(unsigned)
            .ok_or_else(|| ExecutionError::Invalid("aggregate expression overflow".into()))
            .and_then(mixed_negative)
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
        (Number::Unsigned(a), Number::Signed(b)) =>
            unsigned_with_signed(a, b, subtract),
        (Number::Signed(a), Number::Unsigned(b)) if subtract =>
            signed_minus_unsigned(a, b),
        (Number::Signed(a), Number::Unsigned(b)) =>
            unsigned_with_signed(b, a, false),
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

#[cfg(test)]
mod mixed_uint128_binary_tests {
    use super::*;

    #[test]
    fn full_width_unsigned_and_signed_offsets_preserve_exact_results() {
        assert!(matches!(checked_binary(Number::Unsigned(u128::MAX), Number::Signed(-1), false),
            Ok(Number::Unsigned(value)) if value == u128::MAX - 1));
        assert!(matches!(checked_binary(Number::Unsigned(u128::MAX), Number::Signed(1), true),
            Ok(Number::Unsigned(value)) if value == u128::MAX - 1));
        assert!(checked_binary(Number::Unsigned(u128::MAX), Number::Signed(1), false).is_err());
        assert!(matches!(checked_binary(Number::Signed(-1), Number::Unsigned(u128::MAX), false),
            Ok(Number::Unsigned(value)) if value == u128::MAX - 1));
        assert!(matches!(checked_binary(Number::Unsigned(0), Number::Signed(1), true),
            Ok(Number::Signed(-1))));
        assert!(matches!(checked_binary(Number::Signed(0), Number::Unsigned(1), true),
            Ok(Number::Signed(-1))));
        assert!(matches!(checked_binary(Number::Unsigned(0), Number::Signed(i128::MIN), false),
            Ok(Number::Signed(i128::MIN))));
        assert!(checked_binary(Number::Signed(i128::MIN), Number::Unsigned(1), true).is_err());
    }
}
