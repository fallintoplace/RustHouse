//! Strict UTC date and timestamp parsing for stored Date and DateTime columns.

fn days_in_month(year: i64, month: i64) -> i64 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn parse_iso_date_components(text: &str) -> Option<(i64, i64, i64)> {
    let bytes = text.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return None;
    }
    let year = text[..4].parse::<i64>().ok()?;
    let month = text[5..7].parse::<i64>().ok()?;
    let day = text[8..10].parse::<i64>().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    (day >= 1 && day <= days_in_month(year, month)).then_some((year, month, day))
}

fn to_days(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn parse_calendar_day(text: &str) -> Option<i64> {
    let (year, month, day) = parse_iso_date_components(text)?;
    Some(to_days(year, month, day))
}

/// Signed Gregorian day count used by ClickHouse Date32, including dates
/// before the Unix epoch. No timezone or floating-point conversion.
pub fn parse_date32(text: &str) -> Option<i32> {
    i32::try_from(parse_calendar_day(text)?).ok()
}

pub fn format_date32(days: i32) -> String {
    format_calendar_day(i64::from(days))
}

pub fn parse_date(text: &str) -> Option<u16> {
    u16::try_from(parse_calendar_day(text)?).ok()
}

fn parse_clock_time(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 8
        || bytes[2] != b':'
        || bytes[5] != b':'
        || bytes[..2].iter().any(|byte| !byte.is_ascii_digit())
        || bytes[3..5].iter().any(|byte| !byte.is_ascii_digit())
        || bytes[6..].iter().any(|byte| !byte.is_ascii_digit())
    {
        return None;
    }
    let hour = text[..2].parse::<u64>().ok()?;
    let minute = text[3..5].parse::<u64>().ok()?;
    let second = text[6..].parse::<u64>().ok()?;
    (hour <= 23 && minute <= 59 && second <= 59).then_some(hour * 3_600 + minute * 60 + second)
}

fn parse_utc_offset(suffix: &str) -> Option<i64> {
    Some(match suffix {
        "" | "Z" | "z" => 0i64,
        suffix => {
            let bytes = suffix.as_bytes();
            if bytes.len() != 6
                || !matches!(bytes[0], b'+' | b'-')
                || bytes[3] != b':'
                || ![1, 2, 4, 5].iter().all(|&i| bytes[i].is_ascii_digit())
            {
                return None;
            }
            let hours = suffix.get(1..3)?.parse::<i64>().ok()?;
            let minutes = suffix.get(4..6)?.parse::<i64>().ok()?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let seconds = hours * 3_600 + minutes * 60;
            if bytes[0] == b'+' { seconds } else { -seconds }
        }
    })
}

/// Parse a DateTime as UTC, accepting the usual SQL form and RFC 3339
/// whole-second timestamps. Explicit offsets are normalized to UTC before
/// checking the unsigned DateTime storage range.
pub fn parse_datetime(text: &str) -> Option<u32> {
    let timestamp = text.get(..19)?;
    let bytes = timestamp.as_bytes();
    if !matches!(bytes[10], b' ' | b'T' | b't') {
        return None;
    }
    let offset = parse_utc_offset(text.get(19..)?)?;
    let days = parse_calendar_day(text.get(..10)?)?;
    let time = i64::try_from(parse_clock_time(text.get(11..19)?)?).ok()?;
    let seconds = days.checked_mul(86_400)?.checked_add(time)?;
    u32::try_from(seconds.checked_sub(offset)?).ok()
}

fn format_calendar_day(days: i64) -> String {
    let (year, month, day) = from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn format_date(days: u16) -> String {
    format_calendar_day(i64::from(days))
}

pub fn format_datetime(seconds: u32) -> String {
    let days = seconds / 86_400;
    let remainder = seconds % 86_400;
    // Format the calendar date and time together rather than allocating
    // an intermediate String for every timestamp in a result batch.
    let (year, month, day) = from_days(i64::from(days));
    let hour = remainder / 3_600;
    let minute = remainder / 60 % 60;
    let second = remainder % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_centuries_use_gregorian_rules() {
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2025, 2), 28);
        assert_eq!(days_in_month(2025, 4), 30);
    }

    #[test]
    fn dates_are_lossless_and_calendar_checked() {
        for text in ["1970-01-01", "2000-02-29", "2013-07-01", "2149-06-06"] {
            assert_eq!(format_date(parse_date(text).unwrap()), text);
        }
        for text in ["1969-12-31", "2013-02-29", "1900-02-29", "2149-06-07"] {
            assert!(parse_date(text).is_none(), "{text}");
        }
    }

    #[test]
    fn rfc3339_offsets_normalize_to_utc() {
        let epoch = parse_datetime("1970-01-01 00:00:00").unwrap();
        assert_eq!(parse_datetime("1970-01-01T00:00:00Z"), Some(epoch));
        assert_eq!(parse_datetime("1970-01-01T01:30:00+01:30"), Some(epoch));
        assert_eq!(parse_datetime("1969-12-31T22:00:00-02:00"), Some(epoch));
        assert_eq!(
            parse_datetime("2000-01-01T00:00:00+01:00"),
            parse_datetime("1999-12-31 23:00:00")
        );
        assert_eq!(
            parse_datetime("2024-02-29T23:59:59-00:01"),
            parse_datetime("2024-03-01 00:00:59")
        );
        for invalid in [
            "1970-01-01T00:00:00+24:00",
            "1970-01-01T00:00:00+01:60",
            "1970-01-01T00:00:00+01",
            "1970-01-01T00:00:00Zx",
            "1970-01-01T00:00:00.123Z",
            "1970-01-01T00:00:00+00:01",
            "2106-02-07T06:28:15-00:01",
        ] {
            assert_eq!(parse_datetime(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn combined_timestamp_format_matches_independent_calendar_format() {
        for seconds in [0u32, 1, 86_399, 86_400, 1_700_000_000, u32::MAX] {
            let days = seconds / 86_400;
            let remainder = seconds % 86_400;
            let expected = format!(
                "{} {:02}:{:02}:{:02}",
                format_calendar_day(i64::from(days)),
                remainder / 3_600,
                remainder / 60 % 60,
                remainder % 60,
            );
            assert_eq!(format_datetime(seconds), expected);
        }
    }

    #[test]
    fn timestamps_are_lossless_in_utc() {
        for text in [
            "1970-01-01 00:00:00",
            "2000-02-29 23:59:59",
            "2013-07-01 12:34:56",
            "2106-02-07 06:28:15",
        ] {
            assert_eq!(format_datetime(parse_datetime(text).unwrap()), text);
        }
        for text in [
            "1969-12-31 23:59:59",
            "2013-02-29 12:00:00",
            "2024-01-01 24:00:00",
            "2106-02-07 06:28:16",
        ] {
            assert!(parse_datetime(text).is_none(), "{text}");
        }
    }
    #[test]
    fn accepts_lowercase_rfc3339_separators() {
        assert_eq!(
            parse_datetime("2024-02-29t12:30:00z"),
            parse_datetime("2024-02-29T12:30:00Z")
        );
        assert_eq!(
            parse_datetime("2024-02-29t13:30:00+01:00"),
            parse_datetime("2024-02-29T12:30:00Z")
        );
    }
}
