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

fn ymd(text: &str) -> Option<(i64, i64, i64)> {
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
    let (year, month, day) = ymd(text)?;
    Some(to_days(year, month, day))
}

pub fn parse_date(text: &str) -> Option<u16> {
    u16::try_from(parse_calendar_day(text)?).ok()
}

pub fn parse_datetime(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    if bytes.len() != 19
        || !text.is_char_boundary(10)
        || !matches!(bytes[10], b' ' | b'T')
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[11..13].iter().any(|byte| !byte.is_ascii_digit())
        || bytes[14..16].iter().any(|byte| !byte.is_ascii_digit())
        || bytes[17..19].iter().any(|byte| !byte.is_ascii_digit())
    {
        return None;
    }
    let days = u64::try_from(parse_calendar_day(&text[..10])?).ok()?;
    let hour = text[11..13].parse::<u64>().ok()?;
    let minute = text[14..16].parse::<u64>().ok()?;
    let second = text[17..19].parse::<u64>().ok()?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?;
    u32::try_from(seconds).ok()
}

pub fn format_date(days: u16) -> String {
    let (year, month, day) = from_days(i64::from(days));
    format!("{year:04}-{month:02}-{day:02}")
}

pub fn format_datetime(seconds: u32) -> String {
    let days = seconds / 86_400;
    let remainder = seconds % 86_400;
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
}
