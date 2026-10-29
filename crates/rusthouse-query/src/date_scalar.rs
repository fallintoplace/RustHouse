//! UTC-only, bounded date-time compatibility for constant SELECT expressions.
//! Does not yet provide typed Date/DateTime values or column expressions.
use super::{ConstantValue, ParseError, Parser, Token};

const DAY_SECONDS: i64 = 86_400;
const MIN_YEAR: i64 = 1900;
const MAX_YEAR: i64 = 2299;

pub(super) fn is_function(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "ADDDAYS"
            | "TOWEEK"
            | "DATEDIFF"
            | "DATE_DIFF"
            | "TIMEDIFF"
            | "FROMDAYSSINCEYEARZERO32"
            | "TOISOYEAR"
            | "TOISOWEEK"
            | "TOUNIXTIMESTAMP64NANO"
            | "TOUNIXTIMESTAMP64MICRO"
            | "TOUNIXTIMESTAMP64MILLI"
            | "TOYYYYMMDDHHMMSS"
            | "TOYYYYMMDD"
            | "TOYYYYMM"
            | "TOQUARTER"
            | "MONTHNAME"
            | "TOMONTH"
            | "TODAYOFYEAR"
            | "TODAYOFWEEK"
            | "TODAYOFMONTH"
            | "TOLASTDAYOFMONTH"
            | "TOSTARTOFYEAR"
            | "TOSTARTOFQUARTER"
            | "TOSTARTOFMONTH"
            | "TOMONDAY"
            | "TOSTARTOFWEEK"
            | "TOSTARTOFHOUR"
            | "TOSTARTOFDAY"
            | "SUBTRACTQUARTERS"
            | "ADDQUARTERS"
            | "SUBTRACTYEARS"
            | "ADDYEARS"
            | "SUBTRACTSECONDS"
            | "ADDSECONDS"
            | "ADDMINUTES"
            | "ADDHOURS"
            | "SUBTRACTWEEKS"
            | "ADDWEEKS"
            | "SUBTRACTDAYS"
    )
}

pub(super) fn parse_after_open(
    parser: &mut Parser,
    name: &str,
    depth: usize,
) -> Result<ConstantValue, ParseError> {
    match name {
        "TOWEEK" => unary_date(parser, depth, |value| {
            Ok({
                let days = value.seconds.div_euclid(DAY_SECONDS);
                let (year, _, _) = civil_from_days(days);
                let jan_one = days_from_civil(year, 1, 1);
                let first_sunday = jan_one + (7 - (jan_one + 4).rem_euclid(7)).rem_euclid(7);
                let week = if days < first_sunday {
                    0
                } else {
                    (days - first_sunday) / 7 + 1
                };
                ConstantValue::Integer(week)
            })
        }),
        "ADDDAYS" => offset_seconds(parser, depth, DAY_SECONDS, false, false),
        "SUBTRACTDAYS" => offset_seconds(parser, depth, DAY_SECONDS, true, false),
        "ADDWEEKS" => offset_seconds(parser, depth, 7 * DAY_SECONDS, false, false),
        "SUBTRACTWEEKS" => offset_seconds(parser, depth, 7 * DAY_SECONDS, true, false),
        "ADDHOURS" => offset_seconds(parser, depth, 3_600, false, true),
        "ADDMINUTES" => offset_seconds(parser, depth, 60, false, true),
        "ADDSECONDS" => offset_seconds(parser, depth, 1, false, true),
        "SUBTRACTSECONDS" => offset_seconds(parser, depth, 1, true, true),
        "ADDYEARS" => offset_months(parser, depth, 12, false),
        "SUBTRACTYEARS" => offset_months(parser, depth, 12, true),
        "ADDQUARTERS" => offset_months(parser, depth, 3, false),
        "SUBTRACTQUARTERS" => offset_months(parser, depth, 3, true),
        "TOSTARTOFDAY" => unary_date(parser, depth, |value| {
            let seconds = value.seconds.div_euclid(DAY_SECONDS) * DAY_SECONDS;
            Ok(ConstantValue::Text(Temporal::render(seconds, true)?))
        }),
        "TOSTARTOFHOUR" => unary_date(parser, depth, |value| {
            let seconds = value.seconds.div_euclid(3_600) * 3_600;
            Ok(ConstantValue::Text(Temporal::render(seconds, true)?))
        }),
        "TOSTARTOFWEEK" => unary_date(parser, depth, |value| {
            let days = value.seconds.div_euclid(DAY_SECONDS);
            let sunday = days - (days + 4).rem_euclid(7);
            Ok(ConstantValue::Text(Temporal::render(sunday * DAY_SECONDS, false)?))
        }),
        "TOMONDAY" => unary_date(parser, depth, |value| {
            let days = value.seconds.div_euclid(DAY_SECONDS);
            let monday = days - (days + 3).rem_euclid(7);
            Ok(ConstantValue::Text(Temporal::render(monday * DAY_SECONDS, false)?))
        }),
        "TOSTARTOFMONTH" => unary_date(parser, depth, |value| {
            let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let days = days_from_civil(year, month, 1);
            Ok(ConstantValue::Text(Temporal::render(days * DAY_SECONDS, false)?))
        }),
        "TOSTARTOFQUARTER" => unary_date(parser, depth, |value| {
            let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let quarter_month = (month - 1) / 3 * 3 + 1;
            let days = days_from_civil(year, quarter_month, 1);
            Ok(ConstantValue::Text(Temporal::render(days * DAY_SECONDS, false)?))
        }),
        "TOSTARTOFYEAR" => unary_date(parser, depth, |value| {
            let (year, _, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let days = days_from_civil(year, 1, 1);
            Ok(ConstantValue::Text(Temporal::render(days * DAY_SECONDS, false)?))
        }),
        "TOLASTDAYOFMONTH" => unary_date(parser, depth, |value| {
            let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let last = days_in_month(year, month);
            let days = days_from_civil(year, month, last);
            Ok(ConstantValue::Text(Temporal::render(days * DAY_SECONDS, false)?))
        }),
        "TODAYOFMONTH" => unary_date(parser, depth, |value| {
            let (_, _, day) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            Ok(ConstantValue::Integer(i64::from(day)))
        }),
        "TODAYOFWEEK" => unary_date(parser, depth, |value| {
            let days = value.seconds.div_euclid(DAY_SECONDS);
            Ok(ConstantValue::Integer((days + 3).rem_euclid(7) + 1))
        }),
        "TODAYOFYEAR" => unary_date(parser, depth, |value| {
            let days = value.seconds.div_euclid(DAY_SECONDS);
            let (year, _, _) = civil_from_days(days);
            let ordinal = days - days_from_civil(year, 1, 1) + 1;
            Ok(ConstantValue::Integer(ordinal))
        }),
        "TOMONTH" => unary_date(parser, depth, |value| {
            let (_, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            Ok(ConstantValue::Integer(i64::from(month)))
        }),
        "TOQUARTER" => unary_date(parser, depth, |value| {
            let (_, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            Ok(ConstantValue::Integer(i64::from((month - 1) / 3 + 1)))
        }),
        "TOYYYYMM" => unary_date(parser, depth, |value| {
            let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            Ok(ConstantValue::Integer(year * 100 + i64::from(month)))
        }),
        "TOYYYYMMDD" => unary_date(parser, depth, |value| {
            let (year, month, day) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let key = year * 10_000 + i64::from(month) * 100 + i64::from(day);
            Ok(ConstantValue::Integer(key))
        }),
        "TOYYYYMMDDHHMMSS" => unary_date(parser, depth, |value| {
            let (year, month, day) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let seconds = value.seconds.rem_euclid(DAY_SECONDS);
            let ymd = year * 10_000 + i64::from(month) * 100 + i64::from(day);
            let key = ymd * 1_000_000
                + (seconds / 3_600) * 10_000
                + (seconds / 60 % 60) * 100
                + seconds % 60;
            Ok(ConstantValue::Integer(key))
        }),
        "TOISOWEEK" => unary_date(parser, depth, |value| {
            let days = value.seconds.div_euclid(DAY_SECONDS);
            let (_, week) = iso_week_and_year(days);
            Ok(ConstantValue::Integer(week))
        }),
        "TOISOYEAR" => unary_date(parser, depth, |value| {
            let days = value.seconds.div_euclid(DAY_SECONDS);
            let (year, _) = iso_week_and_year(days);
            Ok(ConstantValue::Integer(year))
        }),
        "DATEDIFF" | "DATE_DIFF" => {
            let unit = super::text_scalar::text_arg(parser, depth + 1)?;
            if !parser.take(Token::Comma) {
                return Err(ParseError("dateDiff requires a start date".into()));
            }
            let start = super::text_scalar::text_arg(parser, depth + 1)?;
            if !parser.take(Token::Comma) {
                return Err(ParseError("dateDiff requires an end date".into()));
            }
            let end = super::text_scalar::text_arg(parser, depth + 1)?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("dateDiff requires three arguments".into()));
            }
            let (Some(unit), Some(start), Some(end)) = (unit, start, end) else {
                return Ok(ConstantValue::Null);
            };
            let start = Temporal::parse(&start)?;
            let end = Temporal::parse(&end)?;
            let start_days = start.seconds.div_euclid(DAY_SECONDS);
            let end_days = end.seconds.div_euclid(DAY_SECONDS);
            let difference = match unit.to_ascii_lowercase().as_str() {
                "second" | "seconds" | "s" => end.seconds - start.seconds,
                "minute" | "minutes" => {
                    end.seconds.div_euclid(60) - start.seconds.div_euclid(60)
                }
                "hour" | "hours" => {
                    end.seconds.div_euclid(3_600) - start.seconds.div_euclid(3_600)
                }
                "day" | "days" | "d" => end_days - start_days,
                "week" | "weeks" => {
                    (end_days + 3).div_euclid(7) - (start_days + 3).div_euclid(7)
                }
                "month" | "months" => {
                    let (sy, sm, _) = civil_from_days(start_days);
                    let (ey, em, _) = civil_from_days(end_days);
                    ey * 12 + i64::from(em) - sy * 12 - i64::from(sm)
                }
                "quarter" | "quarters" => {
                    let (sy, sm, _) = civil_from_days(start_days);
                    let (ey, em, _) = civil_from_days(end_days);
                    ey * 4 + i64::from((em - 1) / 3)
                        - sy * 4 - i64::from((sm - 1) / 3)
                }
                "year" | "years" => {
                    let (sy, _, _) = civil_from_days(start_days);
                    let (ey, _, _) = civil_from_days(end_days);
                    ey - sy
                }
                _ => return Err(ParseError("unsupported dateDiff unit".into())),
            };
            Ok(ConstantValue::Integer(difference))
        },
        "MONTHNAME" => unary_date(parser, depth, |value| {
            let (_, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            const MONTHS: &[&str; 12] = &[
                "January", "February", "March", "April",
                "May", "June", "July", "August",
                "September", "October", "November", "December",
            ];
            Ok(ConstantValue::Text(MONTHS[(month - 1) as usize].to_owned()))
        }),
        "TOUNIXTIMESTAMP64NANO" => unary_date(parser, depth, |value| {
            let scaled = value
                .seconds
                .checked_mul(1_000_000_000)
                .ok_or_else(|| ParseError("TOUNIXTIMESTAMP64NANO overflows Int64".into()))?;
            Ok(ConstantValue::Integer(scaled))
        }),
        "TOUNIXTIMESTAMP64MICRO" => unary_date(parser, depth, |value| {
            let scaled = value
                .seconds
                .checked_mul(1_000_000)
                .ok_or_else(|| ParseError("TOUNIXTIMESTAMP64MICRO overflows Int64".into()))?;
            Ok(ConstantValue::Integer(scaled))
        }),
        "TOUNIXTIMESTAMP64MILLI" => unary_date(parser, depth, |value| {
            let scaled = value
                .seconds
                .checked_mul(1_000)
                .ok_or_else(|| ParseError("TOUNIXTIMESTAMP64MILLI overflows Int64".into()))?;
            Ok(ConstantValue::Integer(scaled))
        }),
        "FROMDAYSSINCEYEARZERO32" => {
            let days = super::text_scalar::integer_arg(parser, depth + 1)?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("fromDaysSinceYearZero32 requires one argument".into()));
            }
            match days {
                None => Ok(ConstantValue::Null),
                Some(days) => {
                    if !(693_961..=840_057).contains(&days) {
                        return Err(ParseError("Date32 day count is outside 1900..2299".into()));
                    }
                    let unix_days = days - 719_528;
                    let seconds = unix_days * DAY_SECONDS;
                    Ok(ConstantValue::Text(Temporal::render(seconds, false)?))
                }
            }
        },
        "TIMEDIFF" => time_difference(parser, depth),
        _ => Err(ParseError(format!("unsupported date function: {name}"))),
    }
}

#[derive(Clone, Copy)]
struct Temporal {
    seconds: i64,
    has_time: bool,
}

fn date_error() -> ParseError {
    ParseError("expected a valid YYYY-MM-DD or YYYY-MM-DD HH:MM:SS UTC value".into())
}

impl Temporal {
    fn parse(source: &str) -> Result<Self, ParseError> {
        if !source.is_ascii() || !matches!(source.len(), 10 | 19) {
            return Err(date_error());
        }
        let data = source.as_bytes();
        if data[4] != b'-'
            || data[7] != b'-'
            || [0, 1, 2, 3, 5, 6, 8, 9]
                .iter()
                .any(|index| !data[*index].is_ascii_digit())
        {
            return Err(date_error());
        }
        // Separators and ASCII digits were validated above. Decode each pair
        // directly instead of calling the generic integer parser repeatedly.
        let pair = |index: usize| {
            i64::from(data[index] - b'0') * 10 + i64::from(data[index + 1] - b'0')
        };
        let year = pair(0) * 100 + pair(2);
        let month = pair(5) as u32;
        let day = pair(8) as u32;
        if !(MIN_YEAR..=MAX_YEAR).contains(&year)
            || !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
        {
            return Err(date_error());
        }
        let has_time = source.len() == 19;
        let (hour, minute, second) = if has_time {
            if data[10] != b' '
                || data[13] != b':'
                || data[16] != b':'
                || [11, 12, 14, 15, 17, 18]
                    .iter()
                    .any(|index| !data[*index].is_ascii_digit())
            {
                return Err(date_error());
            }
            (pair(11), pair(14), pair(17))
        } else {
            (0, 0, 0)
        };
        if hour > 23 || minute > 59 || second > 59 {
            return Err(date_error());
        }
        let days = days_from_civil(year, month, day);
        Ok(Self {
            seconds: days * DAY_SECONDS + hour * 3_600 + minute * 60 + second,
            has_time,
        })
    }

    fn render(seconds: i64, has_time: bool) -> Result<String, ParseError> {
        let (year, month, day) = civil_from_days(seconds.div_euclid(DAY_SECONDS));
        if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
            return Err(ParseError("date result is outside 1900..2299".into()));
        }
        if !has_time {
            return Ok(format!("{year:04}-{month:02}-{day:02}"));
        }
        let time = seconds.rem_euclid(DAY_SECONDS);
        Ok(format!(
            "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
            time / 3_600,
            time / 60 % 60,
            time % 60
        ))
    }
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

// Integer Gregorian calendar conversion around the Unix epoch.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_in_era = adjusted_year - era * 400;
    let shifted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_in_era =
        year_in_era * 365 + year_in_era / 4 - year_in_era / 100 + day_of_year;
    era * 146_097 + day_in_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted_days = days + 719_468;
    let era = shifted_days.div_euclid(146_097);
    let day_in_era = shifted_days - era * 146_097;
    let year_in_era = (day_in_era - day_in_era / 1_460 + day_in_era / 36_524
        - day_in_era / 146_096)
        / 365;
    let year = year_in_era + era * 400;
    let day_of_year =
        day_in_era - (365 * year_in_era + year_in_era / 4 - year_in_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = shifted_month + if shifted_month < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month as u32, day as u32)
}

fn offset_seconds(
    parser: &mut Parser,
    depth: usize,
    unit_seconds: i64,
    subtract: bool,
    needs_time: bool,
) -> Result<ConstantValue, ParseError> {
    let input = super::text_scalar::text_arg(parser, depth + 1)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError("date arithmetic requires an amount".into()));
    }
    let amount = super::text_scalar::integer_arg(parser, depth + 1)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' after date arithmetic".into()));
    }
    let (Some(source), Some(amount)) = (input, amount) else {
        return Ok(ConstantValue::Null);
    };
    let value = Temporal::parse(&source)?;
    let signed_amount = if subtract {
        -i128::from(amount)
    } else {
        i128::from(amount)
    };
    let seconds = i128::from(value.seconds) + signed_amount * i128::from(unit_seconds);
    let seconds = i64::try_from(seconds)
        .map_err(|_| ParseError("date arithmetic overflows Int64".into()))?;
    Ok(ConstantValue::Text(Temporal::render(
        seconds,
        value.has_time || needs_time,
    )?))
}

fn offset_months(
    parser: &mut Parser,
    depth: usize,
    months_per_unit: i64,
    subtract: bool,
) -> Result<ConstantValue, ParseError> {
    let source = super::text_scalar::text_arg(parser, depth + 1)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError("calendar arithmetic requires an amount".into()));
    }
    let amount = super::text_scalar::integer_arg(parser, depth + 1)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' after calendar arithmetic".into()));
    }
    let (Some(source), Some(amount)) = (source, amount) else {
        return Ok(ConstantValue::Null);
    };
    let value = Temporal::parse(&source)?;
    let (year, month, day) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
    let current = i128::from(year) * 12 + i128::from(month - 1);
    let delta = i128::from(amount) * i128::from(months_per_unit);
    let target = current + if subtract { -delta } else { delta };
    let target_year = target.div_euclid(12);
    if !(i128::from(MIN_YEAR)..=i128::from(MAX_YEAR)).contains(&target_year) {
        return Err(ParseError("calendar result is outside 1900..2299".into()));
    }
    let target_year = i64::try_from(target_year)
        .map_err(|_| ParseError("calendar year is out of range".into()))?;
    let target_month = (target.rem_euclid(12) + 1) as u32;
    let target_day = day.min(days_in_month(target_year, target_month));
    let target_days = days_from_civil(target_year, target_month, target_day);
    let seconds = target_days * DAY_SECONDS + value.seconds.rem_euclid(DAY_SECONDS);
    Ok(ConstantValue::Text(Temporal::render(seconds, value.has_time)?))
}

fn unary_date(
    parser: &mut Parser,
    depth: usize,
    evaluate: impl FnOnce(Temporal) -> Result<ConstantValue, ParseError>,
) -> Result<ConstantValue, ParseError> {
    let source = super::text_scalar::text_arg(parser, depth + 1)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' after date function".into()));
    }
    match source {
        Some(source) => evaluate(Temporal::parse(&source)?),
        None => Ok(ConstantValue::Null),
    }
}

fn iso_week_and_year(days: i64) -> (i64, i64) {
    // ISO weeks start on Monday; the week containing Thursday defines the year.
    let weekday = (days + 3).rem_euclid(7);
    let thursday = days + 3 - weekday;
    let (iso_year, _, _) = civil_from_days(thursday);
    let january_fourth = days_from_civil(iso_year, 1, 4);
    let week_one_monday = january_fourth - (january_fourth + 3).rem_euclid(7);
    (iso_year, (days - week_one_monday).div_euclid(7) + 1)
}

fn time_difference(parser: &mut Parser, depth: usize) -> Result<ConstantValue, ParseError> {
    let start = super::text_scalar::text_arg(parser, depth + 1)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError("timeDiff requires two dates".into()));
    }
    let end = super::text_scalar::text_arg(parser, depth + 1)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' after timeDiff".into()));
    }
    let (Some(start), Some(end)) = (start, end) else {
        return Ok(ConstantValue::Null);
    };
    let start = Temporal::parse(&start)?;
    let end = Temporal::parse(&end)?;
    let seconds = end
        .seconds
        .checked_sub(start.seconds)
        .ok_or_else(|| ParseError("timeDiff exceeds Int64 range".into()))?;
    Ok(ConstantValue::Integer(seconds))
}

#[cfg(test)]
mod utc_compat_tests {
    use super::*;

    fn value(sql: &str) -> ConstantValue {
        match crate::parse_select(sql).expect(sql).projection {
            crate::Projection::Constants(values) if values.len() == 1 => values[0].value.clone(),
            crate::Projection::Literal(v) => ConstantValue::Integer(v),
            projection => panic!("unexpected projection: {projection:?}"),
        }
    }

    #[test]
    fn parses_ascii_temporal_digits_at_calendar_boundaries() {
        let value = Temporal::parse("2024-02-29 23:59:58").unwrap();
        assert_eq!(Temporal::render(value.seconds, true).unwrap(),
            "2024-02-29 23:59:58");
        assert!(Temporal::parse("2024-02-30").is_err());
        assert!(Temporal::parse("2024-02-29 24:00:00").is_err());
    }

    #[test]
    fn subtract_days_handles_leap_dates_and_bounds() {
        for (sql, expected) in [
            ("SELECT subtractDays('2024-03-01', 1)", "2024-02-29"),
            ("SELECT subtractDays('2024-01-01', -1)", "2024-01-02"),
            ("SELECT subtractDays('2024-03-01 01:30:00', 1)", "2024-02-29 01:30:00"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT subtractDays(NULL, 1)"), ConstantValue::Null);
        for sql in [
            "SELECT subtractDays()",
            "SELECT subtractDays('2023-02-29', 1)",
            "SELECT subtractDays('1900-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn add_weeks_handles_utc_and_bounds() {
        for (sql, expected) in [
            ("SELECT addWeeks('2024-02-22', 1)", "2024-02-29"),
            ("SELECT addWeeks('2024-02-29', -1)", "2024-02-22"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT addWeeks(NULL, 1)"), ConstantValue::Null);
        for sql in [
            "SELECT addWeeks()",
            "SELECT addWeeks('2023-02-29', 1)",
            "SELECT addWeeks('2299-12-31', 1)",
            "SELECT addWeeks('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn subtract_weeks_handles_utc_and_bounds() {
        for (sql, expected) in [
            ("SELECT subtractWeeks('2024-03-07', 1)", "2024-02-29"),
            ("SELECT subtractWeeks('2024-02-22', -1)", "2024-02-29"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT subtractWeeks(NULL, 1)"), ConstantValue::Null);
        for sql in [
            "SELECT subtractWeeks()",
            "SELECT subtractWeeks('2023-02-29', 1)",
            "SELECT subtractWeeks('1900-01-01', 1)",
            "SELECT subtractWeeks('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn add_hours_handles_utc_and_bounds() {
        for (sql, expected) in [
            ("SELECT addHours('2024-02-29 23:30:00', 1)", "2024-03-01 00:30:00"),
            ("SELECT addHours('2024-02-29', 1)", "2024-02-29 01:00:00"),
            ("SELECT addHours('2024-02-29 23:30:00', -1)", "2024-02-29 22:30:00"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT addHours(NULL, 1)"), ConstantValue::Null);
        for sql in [
            "SELECT addHours()",
            "SELECT addHours('2023-02-29', 1)",
            "SELECT addHours('2299-12-31 23:59:59', 1)",
            "SELECT addHours('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn add_minutes_handles_utc_and_bounds() {
        for (sql, expected) in [
            ("SELECT addMinutes('2024-02-29 23:59:30', 1)", "2024-03-01 00:00:30"),
            ("SELECT addMinutes('2024-02-29', 1)", "2024-02-29 00:01:00"),
            ("SELECT addMinutes('2024-02-29 23:59:30', -1)", "2024-02-29 23:58:30"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT addMinutes(NULL, 1)"), ConstantValue::Null);
        for sql in [
            "SELECT addMinutes()",
            "SELECT addMinutes('2023-02-29', 1)",
            "SELECT addMinutes('2299-12-31 23:59:59', 1)",
            "SELECT addMinutes('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn add_seconds_handles_utc_and_bounds() {
        for (sql, expected) in [
            ("SELECT addSeconds('2024-02-29 23:59:59', 1)", "2024-03-01 00:00:00"),
            ("SELECT addSeconds('2024-02-29', 1)", "2024-02-29 00:00:01"),
            ("SELECT addSeconds('2024-02-29 23:59:59', -1)", "2024-02-29 23:59:58"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT addSeconds(NULL, 1)"), ConstantValue::Null);
        for sql in [
            "SELECT addSeconds()",
            "SELECT addSeconds('2023-02-29', 1)",
            "SELECT addSeconds('2299-12-31 23:59:59', 1)",
            "SELECT addSeconds('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn subtract_seconds_handles_utc_and_bounds() {
        for (sql, expected) in [
            ("SELECT subtractSeconds('2024-03-01 00:00:00', 1)", "2024-02-29 23:59:59"),
            ("SELECT subtractSeconds('2024-02-29', 1)", "2024-02-28 23:59:59"),
            ("SELECT subtractSeconds('2024-03-01 00:00:00', -1)", "2024-03-01 00:00:01"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT subtractSeconds(NULL, 1)"), ConstantValue::Null);
        for sql in [
            "SELECT subtractSeconds()",
            "SELECT subtractSeconds('2023-02-29', 1)",
            "SELECT subtractSeconds('1900-01-01 00:00:00', 1)",
            "SELECT subtractSeconds('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn add_years_clamps_month_ends_safely() {
        for (sql, expected) in [
            ("SELECT addYears('2024-02-29', 1)", "2025-02-28"),
            ("SELECT addYears('2024-01-31 12:30:05', 1)", "2025-01-31 12:30:05"),
            ("SELECT addYears('2023-02-28', 1)", "2024-02-28"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT addYears(NULL, 1)"), ConstantValue::Null);
        assert_eq!(value("SELECT addYears('2024-02-29', NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT addYears()",
            "SELECT addYears('2023-02-29', 1)",
            "SELECT addYears('2299-12-31', 1)",
            "SELECT addYears('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn subtract_years_clamps_month_ends_safely() {
        for (sql, expected) in [
            ("SELECT subtractYears('2024-02-29', 1)", "2023-02-28"),
            ("SELECT subtractYears('2024-02-29', -4)", "2028-02-29"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT subtractYears(NULL, 1)"), ConstantValue::Null);
        assert_eq!(value("SELECT subtractYears('2024-02-29', NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT subtractYears()",
            "SELECT subtractYears('2023-02-29', 1)",
            "SELECT subtractYears('1900-01-01', 1)",
            "SELECT subtractYears('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn add_quarters_clamps_month_ends_safely() {
        for (sql, expected) in [
            ("SELECT addQuarters('2024-01-31', 1)", "2024-04-30"),
            ("SELECT addQuarters('2024-11-30', 1)", "2025-02-28"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT addQuarters(NULL, 1)"), ConstantValue::Null);
        assert_eq!(value("SELECT addQuarters('2024-02-29', NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT addQuarters()",
            "SELECT addQuarters('2023-02-29', 1)",
            "SELECT addQuarters('2299-11-30', 1)",
            "SELECT addQuarters('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn subtract_quarters_clamps_month_ends_safely() {
        for (sql, expected) in [
            ("SELECT subtractQuarters('2024-05-31', 1)", "2024-02-29"),
            ("SELECT subtractQuarters('2024-02-29', -1)", "2024-05-29"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT subtractQuarters(NULL, 1)"), ConstantValue::Null);
        assert_eq!(value("SELECT subtractQuarters('2024-02-29', NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT subtractQuarters()",
            "SELECT subtractQuarters('2023-02-29', 1)",
            "SELECT subtractQuarters('1900-02-01', 1)",
            "SELECT subtractQuarters('2024-01-01', 1, 2)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn start_day_preserves_calendar_boundaries() {
        for (sql, expected) in [
            ("SELECT toStartOfDay('2024-02-29 12:34:56')", "2024-02-29 00:00:00"),
            ("SELECT toStartOfDay('2024-02-29')", "2024-02-29 00:00:00"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT toStartOfDay(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toStartOfDay()",
            "SELECT toStartOfDay('2023-02-29')",
            "SELECT toStartOfDay('2024-03-06', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn start_hour_preserves_calendar_boundaries() {
        for (sql, expected) in [
            ("SELECT toStartOfHour('2024-02-29 12:34:56')", "2024-02-29 12:00:00"),
            ("SELECT toStartOfHour('2024-02-29')", "2024-02-29 00:00:00"),
            ("SELECT toStartOfHour('2024-03-01 00:00:59')", "2024-03-01 00:00:00"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT toStartOfHour(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toStartOfHour()",
            "SELECT toStartOfHour('2023-02-29')",
            "SELECT toStartOfHour('2024-03-06', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn start_week_preserves_calendar_boundaries() {
        for (sql, expected) in [
            ("SELECT toStartOfWeek('2024-03-06')", "2024-03-03"),
            ("SELECT toStartOfWeek('2024-03-03')", "2024-03-03"),
            ("SELECT toStartOfWeek('2024-03-06 12:30:00')", "2024-03-03"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT toStartOfWeek(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toStartOfWeek()",
            "SELECT toStartOfWeek('2023-02-29')",
            "SELECT toStartOfWeek('2024-03-06', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn to_monday_preserves_calendar_boundaries() {
        for (sql, expected) in [
            ("SELECT toMonday('2024-03-06')", "2024-03-04"),
            ("SELECT toMonday('2024-03-03')", "2024-02-26"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT toMonday(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toMonday()",
            "SELECT toMonday('2023-02-29')",
            "SELECT toMonday('2024-03-06', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn start_month_uses_gregorian_calendar() {
        for (sql, expected) in [
            ("SELECT toStartOfMonth('2024-02-29')", "2024-02-01"),
            ("SELECT toStartOfMonth('2024-12-31 12:34:56')", "2024-12-01"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT toStartOfMonth(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toStartOfMonth()",
            "SELECT toStartOfMonth('2023-02-29')",
            "SELECT toStartOfMonth('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn start_quarter_uses_gregorian_calendar() {
        for (sql, expected) in [
            ("SELECT toStartOfQuarter('2024-05-31')", "2024-04-01"),
            ("SELECT toStartOfQuarter('2024-01-01')", "2024-01-01"),
            ("SELECT toStartOfQuarter('2024-12-31')", "2024-10-01"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT toStartOfQuarter(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toStartOfQuarter()",
            "SELECT toStartOfQuarter('2023-02-29')",
            "SELECT toStartOfQuarter('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn start_year_uses_gregorian_calendar() {
        for (sql, expected) in [
            ("SELECT toStartOfYear('2024-02-29')", "2024-01-01"),
            ("SELECT toStartOfYear('2025-12-31 23:59:59')", "2025-01-01"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT toStartOfYear(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toStartOfYear()",
            "SELECT toStartOfYear('2023-02-29')",
            "SELECT toStartOfYear('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn last_day_month_uses_gregorian_calendar() {
        for (sql, expected) in [
            ("SELECT toLastDayOfMonth('2024-02-01')", "2024-02-29"),
            ("SELECT toLastDayOfMonth('2023-02-15')", "2023-02-28"),
            ("SELECT toLastDayOfMonth('2024-12-31')", "2024-12-31"),
        ] {
            assert_eq!(value(sql), ConstantValue::Text(expected.into()), "{sql}");
        }
        assert_eq!(value("SELECT toLastDayOfMonth(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toLastDayOfMonth()",
            "SELECT toLastDayOfMonth('2023-02-29')",
            "SELECT toLastDayOfMonth('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn day_of_month_extracts_correct_calendar_values() {
        for (sql, expected) in [
            ("SELECT toDayOfMonth('2024-02-29')", 29),
            ("SELECT toDayOfMonth('2024-01-01')", 1),
            ("SELECT toDayOfMonth('2024-12-31 23:59:59')", 31),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toDayOfMonth(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toDayOfMonth()",
            "SELECT toDayOfMonth('2023-02-29')",
            "SELECT toDayOfMonth('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn day_of_week_extracts_correct_calendar_values() {
        for (sql, expected) in [
            ("SELECT toDayOfWeek('2024-03-04')", 1),
            ("SELECT toDayOfWeek('2024-03-03')", 7),
            ("SELECT toDayOfWeek('1970-01-01')", 4),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toDayOfWeek(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toDayOfWeek()",
            "SELECT toDayOfWeek('2023-02-29')",
            "SELECT toDayOfWeek('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn day_of_year_extracts_correct_calendar_values() {
        for (sql, expected) in [
            ("SELECT toDayOfYear('2024-12-31')", 366),
            ("SELECT toDayOfYear('2023-12-31')", 365),
            ("SELECT toDayOfYear('2024-01-01')", 1),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toDayOfYear(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toDayOfYear()",
            "SELECT toDayOfYear('2023-02-29')",
            "SELECT toDayOfYear('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn to_month_extracts_correct_calendar_values() {
        for (sql, expected) in [
            ("SELECT toMonth('2024-02-29')", 2),
            ("SELECT toMonth('2024-12-01')", 12),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toMonth(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toMonth()",
            "SELECT toMonth('2023-02-29')",
            "SELECT toMonth('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn to_quarter_extracts_correct_calendar_values() {
        for (sql, expected) in [
            ("SELECT toQuarter('2024-03-31')", 1),
            ("SELECT toQuarter('2024-04-01')", 2),
            ("SELECT toQuarter('2024-12-31')", 4),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toQuarter(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toQuarter()",
            "SELECT toQuarter('2023-02-29')",
            "SELECT toQuarter('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn to_yyyymm_extracts_correct_calendar_values() {
        for (sql, expected) in [
            ("SELECT toYYYYMM('2024-02-29')", 202402),
            ("SELECT toYYYYMM('2024-12-31')", 202412),
            ("SELECT toYYYYMM('1900-01-01')", 190001),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toYYYYMM(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toYYYYMM()",
            "SELECT toYYYYMM('2023-02-29')",
            "SELECT toYYYYMM('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn to_yyyymmdd_matches_utc_calendar_examples() {
        for (sql, expected) in [
            ("SELECT toYYYYMMDD('2024-02-29')", 20240229),
            ("SELECT toYYYYMMDD('2299-12-31')", 22991231),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toYYYYMMDD(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toYYYYMMDD()",
            "SELECT toYYYYMMDD('2023-02-29')",
            "SELECT toYYYYMMDD('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn to_yyyymmddhhmmss_matches_utc_calendar_examples() {
        for (sql, expected) in [
            ("SELECT toYYYYMMDDhhmmss('2024-02-29 23:59:59')", 20240229235959),
            ("SELECT toYYYYMMDDhhmmss('2024-02-29')", 20240229000000),
            ("SELECT toYYYYMMDDhhmmss('2299-12-31 23:59:59')", 22991231235959),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toYYYYMMDDhhmmss(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toYYYYMMDDhhmmss()",
            "SELECT toYYYYMMDDhhmmss('2023-02-29')",
            "SELECT toYYYYMMDDhhmmss('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn iso_week_matches_utc_calendar_examples() {
        for (sql, expected) in [
            ("SELECT toISOWeek('2021-01-01')", 53),
            ("SELECT toISOWeek('2021-01-04')", 1),
            ("SELECT toISOWeek('2020-12-31')", 53),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toISOWeek(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toISOWeek()",
            "SELECT toISOWeek('2023-02-29')",
            "SELECT toISOWeek('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn iso_year_matches_utc_calendar_examples() {
        for (sql, expected) in [
            ("SELECT toISOYear('2021-01-01')", 2020),
            ("SELECT toISOYear('2021-01-04')", 2021),
            ("SELECT toISOYear('2019-12-30')", 2020),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT toISOYear(NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT toISOYear()",
            "SELECT toISOYear('2023-02-29')",
            "SELECT toISOYear('2024-01-01', 1)",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn time_diff_matches_utc_calendar_examples() {
        for (sql, expected) in [
            ("SELECT timeDiff('2024-02-29 23:59:59', '2024-03-01 00:00:00')", 1),
            ("SELECT timeDiff('2024-03-01 00:00:00', '2024-02-29 23:59:59')", -1),
            ("SELECT timeDiff('2024-02-28', '2024-03-01')", 172800),
        ] {
            assert_eq!(value(sql), ConstantValue::Integer(expected), "{sql}");
        }
        assert_eq!(value("SELECT timeDiff(NULL, '2024-01-01')"), ConstantValue::Null);
        assert_eq!(value("SELECT timeDiff('2024-01-01', NULL)"), ConstantValue::Null);
        for sql in [
            "SELECT timeDiff()",
            "SELECT timeDiff('2023-02-29', '2024-01-01')",
            "SELECT timeDiff('2024-01-01')",
            "SELECT timeDiff('2024-01-01', '2024-01-02', '2024-01-03')",
        ] {
            assert!(crate::parse_select(sql).is_err(), "{sql}");
        }
    }
}
