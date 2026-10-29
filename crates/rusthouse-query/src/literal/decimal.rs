use super::*;

pub(super) fn fixed_decimal(value: &str) -> Result<(i128, usize), ParseError> {
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
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
    Ok((signed, fraction.len()))
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
    }
}
