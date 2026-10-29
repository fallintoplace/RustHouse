//! Small, checked constant expressions used by analytical WHERE predicates.

mod decimal;

use self::decimal::*;

use crate::ParseError;

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn rescale_decimal(
    value: i128,
    current_scale: usize,
    target_scale: usize,
) -> Result<i128, ParseError> {
    let factor = 10_i128
        .checked_pow((target_scale - current_scale) as u32)
        .ok_or_else(|| ParseError("decimal constant scale is too large".into()))?;
    value
        .checked_mul(factor)
        .ok_or_else(|| ParseError("decimal constant overflow".into()))
}

/// Fold decimal literal arithmetic without floating point rounding in bounds.
pub(super) fn arithmetic(left: &str, right: &str, subtract: bool) -> Result<String, ParseError> {
    let (left, left_scale) = fixed_decimal(left)?;
    let (right, right_scale) = fixed_decimal(right)?;
    let scale = left_scale.max(right_scale);
    let left = rescale_decimal(left, left_scale, scale)?;
    let right = rescale_decimal(right, right_scale, scale)?;
    let value = if subtract {
        left.checked_sub(right)
    } else {
        left.checked_add(right)
    }
    .ok_or_else(|| ParseError("decimal constant overflow".into()))?;

    Ok(format_decimal(value, scale))
}

/// Add calendar years while preserving month/day, clamping Feb 29 to Feb 28.
pub(super) fn add_date_years(date: &str, years: i64) -> Result<String, ParseError> {
    if rusthouse_storage::parse_date(date).is_none() {
        return Err(ParseError("invalid DATE constant".into()));
    }
    let year = date[..4]
        .parse::<i64>()
        .map_err(|_| ParseError("invalid DATE year".into()))?;
    let year = year
        .checked_add(years)
        .ok_or_else(|| ParseError("DATE interval overflow".into()))?;
    if !(0..=9999).contains(&year) {
        return Err(ParseError("DATE interval is out of range".into()));
    }
    let month = &date[5..7];
    let day = if month == "02" && &date[8..10] == "29" {
        if is_leap_year(year) { "29" } else { "28" }
    } else {
        &date[8..10]
    };
    let shifted = format!("{year:04}-{month}-{day}");
    if rusthouse_storage::parse_date(&shifted).is_none() {
        return Err(ParseError(
            "DATE interval is outside supported Date range".into(),
        ));
    }
    Ok(shifted)
}

#[cfg(test)]
mod tests {
    use super::{add_date_years, arithmetic};

    #[test]
    fn folds_decimal_bounds_exactly() {
        assert_eq!(arithmetic("0.06", "0.01", true).unwrap(), "0.05");
        assert_eq!(arithmetic("0.06", "0.01", false).unwrap(), "0.07");
        assert_eq!(arithmetic("1", "2.5", true).unwrap(), "-1.5");
        assert_eq!(arithmetic("-1.25", "1.25", false).unwrap(), "0");
        assert!(arithmetic("170141183460469231731687303715884105727", "1", false).is_err());
    }

    #[test]
    fn folds_minimum_int128_without_positive_magnitude_overflow() {
        let minimum = "-170141183460469231731687303715884105728";
        assert_eq!(arithmetic(minimum, "0", false).unwrap(), minimum);
        assert!(arithmetic(minimum, "1", true).is_err());
    }

    #[test]
    fn shifts_calendar_years_and_handles_leap_day() {
        assert_eq!(add_date_years("1994-01-01", 1).unwrap(), "1995-01-01");
        assert_eq!(add_date_years("2000-02-29", 1).unwrap(), "2001-02-28");
        assert_eq!(add_date_years("1995-02-28", -1).unwrap(), "1994-02-28");
        assert!(add_date_years("1994-02-30", 1).is_err());
        assert!(add_date_years("1994-01-01", 100000).is_err());
    }
}
