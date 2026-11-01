use super::*;

pub(super) fn fixed_decimal(value: &str) -> Result<(i128, usize), ParseError> {
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    let (mantissa, exponent) = if let Some(pos) = digits.find(['e', 'E']) {
        let exponent = digits[pos + 1..].parse::<i32>()
            .map_err(|_| ParseError("invalid decimal exponent".into()))?;
        (&digits[..pos], exponent)
    } else {
        (digits, 0)
    };
    let (whole, original_fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    // Redundant fractional zeros should not consume scale or Int128 capacity.
    let fraction = original_fraction.trim_end_matches('0');
    let whole = if whole.is_empty() && fraction.is_empty() && !original_fraction.is_empty() {
        "0"
    } else {
        whole
    };
    if (whole.is_empty() && fraction.is_empty())
        || fraction.len() > 18
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ParseError("invalid decimal constant".into()));
    }
    // A decimal mantissa does not need to be copied into a temporary
    // String. Accumulate the whole and fractional ASCII digit slices
    // directly, with checked signed arithmetic so Int128::MIN stays valid.
    let mut signed = 0i128;
    for digit in whole.bytes().chain(fraction.bytes()) {
        let value = i128::from(digit - b'0');
        signed = signed.checked_mul(10)
            .and_then(|current| {
                if negative { current.checked_sub(value) }
                else { current.checked_add(value) }
            })
            .ok_or_else(|| ParseError("decimal constant exceeds Int128".into()))?;
    }
    // Zero remains exact under any finite decimal exponent. Avoid rejecting
    // it merely because a redundant power of ten exceeds Int128.
    if signed == 0 {
        return Ok((0, 0));
    }
    let remaining_scale = fraction.len() as i64 - i64::from(exponent);
    if remaining_scale >= 0 {
        let scale = usize::try_from(remaining_scale)
            .map_err(|_| ParseError("decimal constant scale is too large".into()))?;
        if scale > 18 {
            return Err(ParseError("decimal constant scale is too large".into()));
        }
        Ok((signed, scale))
    } else {
        let shift = u32::try_from(-remaining_scale)
            .map_err(|_| ParseError("decimal exponent is too large".into()))?;
        let value = 10_i128
            .checked_pow(shift)
            .and_then(|factor| signed.checked_mul(factor))
            .ok_or_else(|| ParseError("decimal exponent overflows Int128".into()))?;
        Ok((value, 0))
    }
}

pub(super) fn format_decimal(value: i128, scale: usize) -> String {
    let absolute = value.unsigned_abs();
    // 10^39 exceeds u128; never compute that power for a display scale.
    // The usual <=38 case retains the cheap integer division path.
    let mut formatted = if scale == 0 {
        absolute.to_string()
    } else if scale <= 38 {
        let divisor = 10_u128.pow(scale as u32);
        format!(
            "{}.{:0width$}",
            absolute / divisor,
            absolute % divisor,
            width = scale
        )
    } else {
        let digits = absolute.to_string();
        let mut output = String::with_capacity(scale.saturating_add(2));
        output.push_str("0.");
        output.extend(std::iter::repeat_n('0', scale - digits.len()));
        output.push_str(&digits);
        output
    };
    if scale > 0 {
        while formatted.ends_with('0') {
            formatted.pop();
        }
        if formatted.ends_with('.') {
            formatted.pop();
        }
    }
    if value < 0 {
        formatted.insert(0, '-');
    }
    formatted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_leading_and_trailing_decimal_points() {
        assert_eq!(fixed_decimal(".5").unwrap(), (5, 1));
        assert_eq!(fixed_decimal("-.5").unwrap(), (-5, 1));
        assert_eq!(fixed_decimal("1.").unwrap(), (1, 0));
        assert_eq!(format_decimal(fixed_decimal("-.5").unwrap().0, 1), "-0.5");
        assert!(fixed_decimal(".").is_err());
        assert!(fixed_decimal("-.").is_err());
        assert!(fixed_decimal("").is_err());
        assert_eq!(fixed_decimal("1.25e2").unwrap(), (125, 0));
        assert_eq!(fixed_decimal("1.25E-2").unwrap(), (125, 4));
        assert_eq!(fixed_decimal("1.e2").unwrap(), (100, 0));
        assert_eq!(fixed_decimal("-.5e1").unwrap(), (-5, 0));
        assert!(fixed_decimal("1e-19").is_err());
        assert!(fixed_decimal("1e100").is_err());
        assert!(fixed_decimal("1e+").is_err());
        assert_eq!(fixed_decimal("0e500").unwrap(), (0, 0));
        assert_eq!(fixed_decimal("-0.000e-500").unwrap(), (0, 0));
        assert!(fixed_decimal("1e500").is_err());
        assert_eq!(fixed_decimal("1.000000000000000000000000").unwrap(), (1, 0));
        assert_eq!(fixed_decimal(".000000000").unwrap(), (0, 0));
        assert_eq!(fixed_decimal("-.000e-500").unwrap(), (0, 0));
        assert_eq!(fixed_decimal("1.230000000000000000000").unwrap(), (123, 2));
        assert_eq!(fixed_decimal("-0.001000000000000000000").unwrap(), (-1, 3));
        assert_eq!(fixed_decimal("1.23000000e-2").unwrap(), (123, 4));
    }
}

#[cfg(test)]
mod allocation_free_decimal_mantissa_tests {
    use super::*;

    #[test]
    fn parses_int128_extremes_and_exponents_without_mantissa_buffer() {
        for value in [i128::MIN, i128::MIN + 1, -100, -1, 0, 1, 100, i128::MAX - 1, i128::MAX] {
            let repr = value.to_string();
            assert_eq!(fixed_decimal(&repr).unwrap(), (value, 0));
            assert_eq!(fixed_decimal(&format!("{repr}.0000")).unwrap(), (value, 0));
        }
        for repr in [
            "170141183460469231731687303715884105728",
            "-170141183460469231731687303715884105729",
            "99999999999999999999999999999999999999999999",
        ] {
            assert!(fixed_decimal(repr).is_err(), "{repr}");
        }
        for (repr, expected) in [
            ("-.125e2", (-125i128, 1usize)),
            ("0.00123000e2", (123, 3)),
            ("-1.0000000e-2", (-1, 2)),
            ("+1.25", (125, 2)),
        ] {
            if repr.starts_with('+') {
                assert!(fixed_decimal(repr).is_err());
            } else {
                assert_eq!(fixed_decimal(repr).unwrap(), expected, "{repr}");
            }
        }
    }
}

#[cfg(test)]
mod wide_display_scale_tests {
    use super::*;

    #[test]
    fn formatting_large_scales_does_not_overflow_u128() {
        assert_eq!(format_decimal(1, 39), format!("0.{}1", "0".repeat(38)));
        assert_eq!(format_decimal(-10, 40), format!("-0.{}1", "0".repeat(38)));
        assert_eq!(format_decimal(0, 40), "0");
        assert_eq!(format_decimal(i128::MIN, 39).split('.').next(), Some("-0"));
        for scale in [0usize, 1, 18, 38] {
            assert_eq!(format_decimal(100, scale).starts_with('-'), false);
        }
    }
}
