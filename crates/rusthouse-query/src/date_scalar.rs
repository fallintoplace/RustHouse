//! UTC-only, bounded date-time compatibility for constant SELECT expressions.
//! Does not yet provide typed Date/DateTime values or column expressions.
use super::{ConstantValue, ParseError, Parser, Token};

const DAY_SECONDS: i64 = 86_400;
const MIN_YEAR: i64 = 1900;
const MAX_YEAR: i64 = 2299;

pub(super) fn render_datetime64_utc(seconds: i64) -> Result<String, ParseError> {
    Temporal::render(seconds, true)
}

pub(super) fn is_function(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "MAKEDATE"
            | "MAKEDATETIME"
            | "TOSTARTOFHALFHOUR"
            | "ADDDAYS"
            | "TOHOUR"
            | "TOMINUTE"
            | "TOSECOND"
            | "TODATE"
            | "TODATETIME"
            | "TORELATIVEMONTHNUM"
            | "TORELATIVEWEEKNUM"
            | "TORELATIVEDAYNUM"
            | "TORELATIVEHOURNUM"
            | "TORELATIVEMINUTENUM"
            | "TORELATIVESECONDNUM"
            | "TOLASTDAYOFYEAR"
            | "TOLASTDAYOFQUARTER"
            | "TOWEEK"
            | "ISLEAPYEAR"
            | "TODAYSINMONTH"
            | "DAYSINMONTH"
            | "TIMESLOT"
            | "TORELATIVEYEARNUM"
            | "TORELATIVEQUARTERNUM"
            | "TOYEARWEEK"
            | "DATEDIFF"
            | "DATE_DIFF"
            | "TIMEDIFF"
            | "FROMDAYSSINCEYEARZERO32"
            | "FROMDAYSSINCEYEARZERO"
            | "FROM_DAYS"
            | "TODAYSSINCEYEARZERO"
            | "TO_DAYS"
            | "TOISOYEAR"
            | "TOISOWEEK"
            | "TOUNIXTIMESTAMP64SECOND"
            | "TOUNIXTIMESTAMP64NANO"
            | "TOUNIXTIMESTAMP64MICRO"
            | "TOUNIXTIMESTAMP64MILLI"
            | "TOYYYYMMDDHHMMSS"
            | "TOYYYYMMDD"
            | "TOYYYYMM"
            | "TOQUARTER"
            | "DATENAME"
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
        "MAKEDATE" => {
            let year = super::text_scalar::integer_arg(parser, depth + 1)?;
            require_separator(parser, "makeDate requires year, month and day")?;
            let month = super::text_scalar::integer_arg(parser, depth + 1)?;
            require_separator(parser, "makeDate requires year, month and day")?;
            let day = super::text_scalar::integer_arg(parser, depth + 1)?;
            if !parser.take(Token::RParen) {
                return Err(ParseError(
                    "makeDate requires exactly three arguments".into(),
                ));
            }
            match (year, month, day) {
                (Some(year), Some(month), Some(day)) => {
                    let month = u32::try_from(month)
                        .map_err(|_| ParseError("invalid makeDate month".into()))?;
                    let day = u32::try_from(day)
                        .map_err(|_| ParseError("invalid makeDate day".into()))?;
                    if !(MIN_YEAR..=MAX_YEAR).contains(&year)
                        || !(1..=12).contains(&month)
                        || day == 0
                        || day > days_in_month(year, month)
                    {
                        return Err(ParseError("invalid makeDate calendar date".into()));
                    }
                    let seconds = days_from_civil(year, month, day) * DAY_SECONDS;
                    Ok(ConstantValue::Text(Temporal::render(seconds, false)?))
                }
                _ => Ok(ConstantValue::Null),
            }
        }
        "MAKEDATETIME" => {
            let mut values = [None; 6];
            for (index, slot) in values.iter_mut().enumerate() {
                if index > 0 {
                    require_separator(parser, "makeDateTime requires six integer fields")?;
                }
                *slot = super::text_scalar::integer_arg(parser, depth + 1)?;
            }
            if !parser.take(Token::RParen) {
                return Err(ParseError(
                    "makeDateTime requires exactly six arguments".into(),
                ));
            }
            if values.iter().any(Option::is_none) {
                return Ok(ConstantValue::Null);
            }
            let [year, month, day, hour, minute, second] = values.map(Option::unwrap);
            let month = u32::try_from(month)
                .map_err(|_| ParseError("invalid makeDateTime month".into()))?;
            let day =
                u32::try_from(day).map_err(|_| ParseError("invalid makeDateTime day".into()))?;
            if !(MIN_YEAR..=MAX_YEAR).contains(&year)
                || !(1..=12).contains(&month)
                || day == 0
                || day > days_in_month(year, month)
                || !(0..24).contains(&hour)
                || !(0..60).contains(&minute)
                || !(0..60).contains(&second)
            {
                return Err(ParseError("invalid makeDateTime UTC fields".into()));
            }
            let seconds = days_from_civil(year, month, day) * DAY_SECONDS
                + hour * 3_600
                + minute * 60
                + second;
            Ok(ConstantValue::Text(Temporal::render(seconds, true)?))
        }
        "TOSTARTOFHALFHOUR" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Text(Temporal::render(
                value.seconds.div_euclid(1_800) * 1_800,
                true,
            )?))
        }),
        "TOYEARWEEK" => unary_date(parser, depth, |value| {
            Ok({
                let days = value.seconds.div_euclid(DAY_SECONDS);
                let (year, _, _) = civil_from_days(days);
                let jan_one = days_from_civil(year, 1, 1);
                let first_sunday = jan_one + (7 - (jan_one + 4).rem_euclid(7)).rem_euclid(7);
                let (week_year, week) = if days < first_sunday {
                    let previous_year = year - 1;
                    let prev_jan_one = days_from_civil(previous_year, 1, 1);
                    let first_prev_sunday =
                        prev_jan_one + (7 - (prev_jan_one + 4).rem_euclid(7)).rem_euclid(7);
                    (previous_year, (jan_one - 1 - first_prev_sunday) / 7 + 1)
                } else {
                    (year, (days - first_sunday) / 7 + 1)
                };
                ConstantValue::Integer(week_year * 100 + week)
            })
        }),
        "TOLASTDAYOFYEAR" => unary_date(parser, depth, |value| {
            Ok({
                let (year, _, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
                let days = days_from_civil(year, 12, 31);
                ConstantValue::Text(Temporal::render(days * DAY_SECONDS, false)?)
            })
        }),
        "TOLASTDAYOFQUARTER" => unary_date(parser, depth, |value| {
            Ok({
                let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
                let last_month = ((month - 1) / 3 + 1) * 3;
                let last_day = days_in_month(year, last_month);
                let days = days_from_civil(year, last_month, last_day);
                ConstantValue::Text(Temporal::render(days * DAY_SECONDS, false)?)
            })
        }),
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
        "ISLEAPYEAR" => unary_date(parser, depth, |value| {
            Ok({
                let (year, _, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
                ConstantValue::Integer(i64::from(
                    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0),
                ))
            })
        }),
        "TODAYSINMONTH" => unary_date(parser, depth, |value| {
            let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            Ok(ConstantValue::Integer(i64::from(days_in_month(
                year, month,
            ))))
        }),
        "DAYSINMONTH" => unary_date(parser, depth, |value| {
            Ok({
                let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
                ConstantValue::Integer(i64::from(days_in_month(year, month)))
            })
        }),
        "TIMESLOT" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Text(Temporal::render(
                value.seconds.div_euclid(1_800) * 1_800,
                true,
            )?))
        }),
        "TORELATIVEYEARNUM" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer({
                let (year, _, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
                year - 1970
            }))
        }),
        "TORELATIVEQUARTERNUM" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer({
                let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
                (year - 1970) * 4 + i64::from((month - 1) / 3)
            }))
        }),
        "TORELATIVEMONTHNUM" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer({
                let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
                (year - 1970) * 12 + i64::from(month) - 1
            }))
        }),
        "TORELATIVEWEEKNUM" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer(
                (value.seconds.div_euclid(DAY_SECONDS) + 4).div_euclid(7),
            ))
        }),
        "TORELATIVEDAYNUM" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer(
                value.seconds.div_euclid(DAY_SECONDS),
            ))
        }),
        "TORELATIVEHOURNUM" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer(value.seconds.div_euclid(3_600)))
        }),
        "TORELATIVEMINUTENUM" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer(value.seconds.div_euclid(60)))
        }),
        "TORELATIVESECONDNUM" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer(value.seconds))
        }),
        "TOHOUR" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer(
                value.seconds.rem_euclid(DAY_SECONDS) / 3_600,
            ))
        }),
        "TOMINUTE" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer(value.seconds.rem_euclid(3_600) / 60))
        }),
        "TOSECOND" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer(value.seconds.rem_euclid(60)))
        }),
        "TODATE" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Text(Temporal::render(
                value.seconds.div_euclid(DAY_SECONDS) * DAY_SECONDS,
                false,
            )?))
        }),
        "TODATETIME" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Text(Temporal::render(value.seconds, true)?))
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
        "TOSTARTOFWEEK" => {
            let source = super::text_scalar::text_arg(parser, depth + 1)?;
            let mode = if parser.take(Token::Comma) {
                super::text_scalar::integer_arg(parser, depth + 1)?
            } else {
                Some(0)
            };
            if !parser.take(Token::RParen) {
                return Err(ParseError(
                    "toStartOfWeek requires a date and optional mode".into(),
                ));
            }
            match (source, mode) {
                (Some(source), Some(mode)) => {
                    if !(0..=9).contains(&mode) {
                        return Err(ParseError(
                            "toStartOfWeek mode must be between 0 and 9".into(),
                        ));
                    }
                    let value = Temporal::parse(&source)?;
                    let days = value.seconds.div_euclid(DAY_SECONDS);
                    let first_weekday_offset = if mode % 2 == 0 { 4 } else { 3 };
                    let first = days - (days + first_weekday_offset).rem_euclid(7);
                    Ok(ConstantValue::Text(Temporal::render(
                        first * DAY_SECONDS,
                        false,
                    )?))
                }
                _ => Ok(ConstantValue::Null),
            }
        }
        "TOMONDAY" => unary_date(parser, depth, |value| {
            let days = value.seconds.div_euclid(DAY_SECONDS);
            let monday = days - (days + 3).rem_euclid(7);
            Ok(ConstantValue::Text(Temporal::render(
                monday * DAY_SECONDS,
                false,
            )?))
        }),
        "TOSTARTOFMONTH" => unary_date(parser, depth, |value| {
            let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let days = days_from_civil(year, month, 1);
            Ok(ConstantValue::Text(Temporal::render(
                days * DAY_SECONDS,
                false,
            )?))
        }),
        "TOSTARTOFQUARTER" => unary_date(parser, depth, |value| {
            let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let quarter_month = (month - 1) / 3 * 3 + 1;
            let days = days_from_civil(year, quarter_month, 1);
            Ok(ConstantValue::Text(Temporal::render(
                days * DAY_SECONDS,
                false,
            )?))
        }),
        "TOSTARTOFYEAR" => unary_date(parser, depth, |value| {
            let (year, _, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let days = days_from_civil(year, 1, 1);
            Ok(ConstantValue::Text(Temporal::render(
                days * DAY_SECONDS,
                false,
            )?))
        }),
        "TOLASTDAYOFMONTH" => unary_date(parser, depth, |value| {
            let (year, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            let last = days_in_month(year, month);
            let days = days_from_civil(year, month, last);
            Ok(ConstantValue::Text(Temporal::render(
                days * DAY_SECONDS,
                false,
            )?))
        }),
        "TODAYOFMONTH" => unary_date(parser, depth, |value| {
            let (_, _, day) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            Ok(ConstantValue::Integer(i64::from(day)))
        }),
        "TODAYOFWEEK" => {
            let input = super::text_scalar::text_arg(parser, depth + 1)?;
            let mode = if parser.take(Token::Comma) {
                super::text_scalar::integer_arg(parser, depth + 1)?
            } else {
                Some(0)
            };
            if !parser.take(Token::RParen) {
                return Err(ParseError("toDayOfWeek needs one or two arguments".into()));
            }
            let (Some(input), Some(mode)) = (input, mode) else {
                return Ok(ConstantValue::Null);
            };
            let day = (Temporal::parse(&input)?.seconds.div_euclid(DAY_SECONDS) + 3).rem_euclid(7);
            let result = match mode {
                0 => day + 1,                     // Monday 1..7
                1 => day,                         // Monday 0..6
                2 => (day + 1).rem_euclid(7),     // Sunday 0..6
                3 => (day + 1).rem_euclid(7) + 1, // Sunday 1..7
                _ => return Err(ParseError("toDayOfWeek mode must be 0..3".into())),
            };
            Ok(ConstantValue::Integer(result))
        }
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
                "minute" | "minutes" => end.seconds.div_euclid(60) - start.seconds.div_euclid(60),
                "hour" | "hours" => end.seconds.div_euclid(3_600) - start.seconds.div_euclid(3_600),
                "day" | "days" | "d" => end_days - start_days,
                "week" | "weeks" => (end_days + 3).div_euclid(7) - (start_days + 3).div_euclid(7),
                "month" | "months" => {
                    let (sy, sm, _) = civil_from_days(start_days);
                    let (ey, em, _) = civil_from_days(end_days);
                    ey * 12 + i64::from(em) - sy * 12 - i64::from(sm)
                }
                "quarter" | "quarters" => {
                    let (sy, sm, _) = civil_from_days(start_days);
                    let (ey, em, _) = civil_from_days(end_days);
                    ey * 4 + i64::from((em - 1) / 3) - sy * 4 - i64::from((sm - 1) / 3)
                }
                "year" | "years" => {
                    let (sy, _, _) = civil_from_days(start_days);
                    let (ey, _, _) = civil_from_days(end_days);
                    ey - sy
                }
                _ => return Err(ParseError("unsupported dateDiff unit".into())),
            };
            Ok(ConstantValue::Integer(difference))
        }
        "DATENAME" => {
            let part = super::text_scalar::text_arg(parser, depth + 1)?;
            require_separator(parser, "dateName requires a date value")?;
            let date = super::text_scalar::text_arg(parser, depth + 1)?;
            let timezone = if parser.take(Token::Comma) {
                Some(super::text_scalar::text_arg(parser, depth + 1)?)
            } else {
                None
            };
            if !parser.take(Token::RParen) {
                return Err(ParseError("dateName expects two or three arguments".into()));
            }
            let (Some(part), Some(date)) = (part, date) else {
                return Ok(ConstantValue::Null);
            };
            if timezone == Some(None) {
                return Ok(ConstantValue::Null);
            }
            if timezone
                .as_ref()
                .and_then(|zone| zone.as_ref())
                .is_some_and(|zone| zone != "UTC")
            {
                return Err(ParseError("dateName only supports UTC timestamps".into()));
            }
            let date = Temporal::parse(&date)?;
            let days = date.seconds.div_euclid(DAY_SECONDS);
            let (year, month, day) = civil_from_days(days);
            let time = date.seconds.rem_euclid(DAY_SECONDS);
            let result = match part.as_str() {
                "year" => year.to_string(),
                "quarter" => ((month - 1) / 3 + 1).to_string(),
                "month" => {
                    const MONTHS: &[&str; 12] = &[
                        "January",
                        "February",
                        "March",
                        "April",
                        "May",
                        "June",
                        "July",
                        "August",
                        "September",
                        "October",
                        "November",
                        "December",
                    ];
                    MONTHS[(month - 1) as usize].to_owned()
                }
                "week" => iso_week_and_year(days).1.to_string(),
                "dayofyear" => (days - days_from_civil(year, 1, 1) + 1).to_string(),
                "day" => day.to_string(),
                "weekday" => {
                    const WEEKDAYS: &[&str; 7] = &[
                        "Monday",
                        "Tuesday",
                        "Wednesday",
                        "Thursday",
                        "Friday",
                        "Saturday",
                        "Sunday",
                    ];
                    WEEKDAYS[(days + 3).rem_euclid(7) as usize].to_owned()
                }
                "hour" => (time / 3600).to_string(),
                "minute" => (time / 60 % 60).to_string(),
                "second" => (time % 60).to_string(),
                _ => return Err(ParseError("invalid dateName part".into())),
            };
            Ok(ConstantValue::Text(result))
        }
        "MONTHNAME" => unary_date(parser, depth, |value| {
            let (_, month, _) = civil_from_days(value.seconds.div_euclid(DAY_SECONDS));
            const MONTHS: &[&str; 12] = &[
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            Ok(ConstantValue::Text(MONTHS[(month - 1) as usize].to_owned()))
        }),
        "TOUNIXTIMESTAMP64SECOND" => unary_date(parser, depth, |value| {
            Ok(ConstantValue::Integer(value.seconds))
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
                return Err(ParseError(
                    "fromDaysSinceYearZero32 requires one argument".into(),
                ));
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
        }
        "FROMDAYSSINCEYEARZERO" | "FROM_DAYS" => {
            let days = super::text_scalar::integer_arg(parser, depth + 1)?;
            if !parser.take(Token::RParen) {
                return Err(ParseError(
                    "fromDaysSinceYearZero requires one argument".into(),
                ));
            }
            match days {
                None => Ok(ConstantValue::Null),
                Some(days) => {
                    if !(719_528..=785_063).contains(&days) {
                        return Err(ParseError("Date day count is outside 1970..2149".into()));
                    }
                    let unix_days = days - 719_528;
                    let seconds = unix_days * DAY_SECONDS;
                    Ok(ConstantValue::Text(Temporal::render(seconds, false)?))
                }
            }
        }
        "TODAYSSINCEYEARZERO" | "TO_DAYS" => unary_date(parser, depth, |value| {
            let unix_days = value.seconds.div_euclid(DAY_SECONDS);
            Ok(ConstantValue::Integer(unix_days + 719_528))
        }),
        "TIMEDIFF" => time_difference(parser, depth),
        _ => Err(ParseError(format!("unsupported date function: {name}"))),
    }
}

fn require_separator(parser: &mut Parser, message: &str) -> Result<(), ParseError> {
    if parser.take(Token::Comma) {
        Ok(())
    } else {
        Err(ParseError(message.into()))
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
        let pair =
            |index: usize| i64::from(data[index] - b'0') * 10 + i64::from(data[index + 1] - b'0');
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
    let day_in_era = year_in_era * 365 + year_in_era / 4 - year_in_era / 100 + day_of_year;
    era * 146_097 + day_in_era - 719_468
}

pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted_days = days + 719_468;
    let era = shifted_days.div_euclid(146_097);
    let day_in_era = shifted_days - era * 146_097;
    let year_in_era =
        (day_in_era - day_in_era / 1_460 + day_in_era / 36_524 - day_in_era / 146_096) / 365;
    let year = year_in_era + era * 400;
    let day_of_year = day_in_era - (365 * year_in_era + year_in_era / 4 - year_in_era / 100);
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
    let seconds =
        i64::try_from(seconds).map_err(|_| ParseError("date arithmetic overflows Int64".into()))?;
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
    Ok(ConstantValue::Text(Temporal::render(
        seconds,
        value.has_time,
    )?))
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
#[path = "date_scalar_tests/utc_compat_tests.rs"]
mod utc_compat_tests;

#[cfg(test)]
#[path = "date_scalar_tests/clickhouse_start_of_half_hour_tests.rs"]
mod clickhouse_start_of_half_hour_tests;

#[cfg(test)]
#[path = "date_scalar_tests/day_of_week_mode_tests.rs"]
mod day_of_week_mode_tests;

#[cfg(test)]
#[path = "date_scalar_tests/to_days_in_month_tests.rs"]
mod to_days_in_month_tests;

#[cfg(test)]
#[path = "date_scalar_tests/make_date_constant_tests.rs"]
mod make_date_constant_tests;

#[cfg(test)]
#[path = "date_scalar_tests/make_datetime_constant_tests.rs"]
mod make_datetime_constant_tests;
