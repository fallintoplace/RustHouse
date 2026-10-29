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
            | "SUBTRACTDAYS"
    )
}

pub(super) fn parse_after_open(
    parser: &mut Parser,
    name: &str,
    depth: usize,
) -> Result<ConstantValue, ParseError> {
    match name {
        "ADDDAYS" => offset_seconds(parser, depth, DAY_SECONDS, false, false),
        "SUBTRACTDAYS" => offset_seconds(parser, depth, DAY_SECONDS, true, false),
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
        let year = source[0..4].parse::<i64>().map_err(|_| date_error())?;
        let month = source[5..7].parse::<u32>().map_err(|_| date_error())?;
        let day = source[8..10].parse::<u32>().map_err(|_| date_error())?;
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
            (
                source[11..13].parse::<i64>().map_err(|_| date_error())?,
                source[14..16].parse::<i64>().map_err(|_| date_error())?,
                source[17..19].parse::<i64>().map_err(|_| date_error())?,
            )
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
    if needs_time && !value.has_time {
        return Err(ParseError("this function requires a DateTime value".into()));
    }
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
}
