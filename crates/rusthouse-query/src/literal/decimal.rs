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
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    // Redundant fractional zeros should not consume scale or Int128 capacity.
    let fraction = fraction.trim_end_matches('0');
    if (whole.is_empty() && fraction.is_empty())
        || fraction.len() > 18
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ParseError("invalid decimal constant".into()));
    }
    let digits = format!("{whole}{fraction}");
    let signed = if negative {
        format!("-{digits}").parse::<i128>()
    } else {
        digits.parse::<i128>()
    }
    .map_err(|_| ParseError("decimal constant exceeds Int128".into()))?;
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
        let factor = 10_i128.checked_pow(shift)
            .ok_or_else(|| ParseError("decimal exponent overflows Int128".into()))?;
        let value = signed.checked_mul(factor)
            .ok_or_else(|| ParseError("decimal exponent overflows Int128".into()))?;
        Ok((value, 0))
    }
}

pub(super) fn format_decimal(value: i128, scale: usize) -> String {
    let absolute = value.unsigned_abs();
    let divisor = 10_u128.pow(scale as u32);
    let mut formatted = if scale == 0 {
        absolute.to_string()
    } else {
        format!(
            "{}.{:0width$}",
            absolute / divisor,
            absolute % divisor,
            width = scale
        )
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
        assert_eq!(fixed_decimal("1.230000000000000000000").unwrap(), (123, 2));
        assert_eq!(fixed_decimal("-0.001000000000000000000").unwrap(), (-1, 3));
        assert_eq!(fixed_decimal("1.23000000e-2").unwrap(), (123, 4));
    }
}
