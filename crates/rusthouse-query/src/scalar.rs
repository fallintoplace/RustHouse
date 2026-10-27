//! Bounded constant-expression evaluation for scalar SELECT without FROM.
//! Calculation happens in the parser: the executor still sees a single Literal.
use super::{ParseError, Parser, Token};

const MAX_DEPTH: usize = 16;

fn is_extrema(name: &str) -> bool {
    name.eq_ignore_ascii_case("GREATEST") || name.eq_ignore_ascii_case("LEAST")
}

fn is_power(name: &str) -> bool {
    name.eq_ignore_ascii_case("POWER") || name.eq_ignore_ascii_case("POW")
}

fn is_bitwise(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "BITAND"
            | "BITOR"
            | "BITXOR"
            | "BITSHIFTLEFT"
            | "BITSHIFTRIGHT"
            | "BITROTATELEFT"
            | "BITROTATERIGHT"
    )
}

pub(super) fn is_function(name: &str) -> bool {
    name.eq_ignore_ascii_case("ABS")
        || name.eq_ignore_ascii_case("BIT_WIDTH")
        || name.eq_ignore_ascii_case("SIGN")
        || is_extrema(name)
        || is_power(name)
        || is_bitwise(name)
        || name.eq_ignore_ascii_case("GCD")
        || name.eq_ignore_ascii_case("LCM")
        || name.eq_ignore_ascii_case("CLAMP")
        || name.eq_ignore_ascii_case("CEIL_DIV")
        || name.eq_ignore_ascii_case("FLOOR_DIV")
}

pub(super) fn parse(parser: &mut Parser) -> Result<i64, ParseError> {
    expression(parser, 0)
}

fn expression(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let mut value = term(parser, depth)?;
    loop {
        if parser.take(Token::Plus) {
            value = value
                .checked_add(term(parser, depth)?)
                .ok_or_else(overflow)?;
        } else if parser.take(Token::Minus) {
            value = value
                .checked_sub(term(parser, depth)?)
                .ok_or_else(overflow)?;
        } else if matches!(
            parser.tokens.get(parser.index),
            Some(Token::Number(number)) if number.starts_with('-')
        ) {
            // The shared lexer represents "-2" as one signed literal.
            // In scalar expressions this is also a subtraction shorthand.
            value = value
                .checked_add(term(parser, depth)?)
                .ok_or_else(overflow)?;
        } else {
            break;
        }
    }
    Ok(value)
}

fn term(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let mut value = factor(parser, depth)?;
    loop {
        if parser.take(Token::Star) {
            value = value
                .checked_mul(factor(parser, depth)?)
                .ok_or_else(overflow)?;
        } else if parser.take(Token::Slash) {
            value = value
                .checked_div(factor(parser, depth)?)
                .ok_or(ParseError("scalar division by zero or overflow".into()))?;
        } else if parser.take(Token::Percent) {
            value = value
                .checked_rem(factor(parser, depth)?)
                .ok_or(ParseError("scalar modulo by zero or overflow".into()))?;
        } else {
            break;
        }
    }
    Ok(value)
}

fn require_token(parser: &mut Parser, token: Token, message: &str) -> Result<(), ParseError> {
    if parser.take(token) {
        Ok(())
    } else {
        Err(ParseError(message.into()))
    }
}

fn parse_power(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "POWER requires '('")?;
    let base = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "POWER requires two arguments")?;
    let exponent = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "POWER requires ')' after exponent")?;
    let exponent = u32::try_from(exponent)
        .map_err(|_| ParseError("POWER exponent must be non-negative and fit UInt32".into()))?;
    base.checked_pow(exponent).ok_or_else(overflow)
}

fn parse_extrema(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let name = parser.word()?;
    if !parser.take(Token::LParen) {
        return Err(ParseError(format!("{name} requires '('")));
    }
    let mut value = expression(parser, depth + 1)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError(format!(
            "{name} requires at least two arguments"
        )));
    }
    let mut count = 1usize;
    loop {
        if count == 16 {
            return Err(ParseError(format!("{name} supports at most 16 arguments")));
        }
        let candidate = expression(parser, depth + 1)?;
        value = if name.eq_ignore_ascii_case("GREATEST") {
            value.max(candidate)
        } else {
            value.min(candidate)
        };
        count += 1;
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if !parser.take(Token::RParen) {
        return Err(ParseError(format!("expected ')' after {name} arguments")));
    }
    Ok(value)
}

fn parse_bit_width(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "BIT_WIDTH requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "expected ')' after BIT_WIDTH argument",
    )?;
    Ok(i64::from(u64::BITS - value.unsigned_abs().leading_zeros()))
}

fn checked_bit_shift(value: i64, amount: i64, left: bool) -> Result<i64, ParseError> {
    let shift = u32::try_from(amount)
        .map_err(|_| ParseError("bit shift count must be between 0 and 63".into()))?;
    if shift >= i64::BITS {
        return Err(ParseError(
            "bit shift count must be between 0 and 63".into(),
        ));
    }
    if left {
        i64::try_from(i128::from(value) << shift).map_err(|_| overflow())
    } else {
        Ok(value >> shift)
    }
}

fn checked_bit_rotate(value: i64, amount: i64, left: bool) -> Result<i64, ParseError> {
    let shift = u32::try_from(amount)
        .map_err(|_| ParseError("bit rotate count must be between 0 and 63".into()))?;
    if shift >= i64::BITS {
        return Err(ParseError(
            "bit rotate count must be between 0 and 63".into(),
        ));
    }
    Ok(if left {
        value.rotate_left(shift)
    } else {
        value.rotate_right(shift)
    })
}

fn parse_clamp(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    require_token(parser, Token::LParen, "CLAMP requires '('")?;
    let value = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "CLAMP requires three arguments")?;
    let lower = expression(parser, depth + 1)?;
    require_token(parser, Token::Comma, "CLAMP requires three arguments")?;
    let upper = expression(parser, depth + 1)?;
    require_token(parser, Token::RParen, "expected ')' after CLAMP arguments")?;
    if lower > upper {
        return Err(ParseError("CLAMP lower bound exceeds upper bound".into()));
    }
    Ok(value.clamp(lower, upper))
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn parse_gcd_lcm(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let name = parser.word()?;
    if !parser.take(Token::LParen) {
        return Err(ParseError(format!("{name} requires '('")));
    }
    let left = expression(parser, depth + 1)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError(format!("{name} requires two arguments")));
    }
    let right = expression(parser, depth + 1)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError(format!("expected ')' after {name} arguments")));
    }
    let left = left.unsigned_abs();
    let right = right.unsigned_abs();
    let result = if name.eq_ignore_ascii_case("GCD") {
        gcd(left, right)
    } else if left == 0 || right == 0 {
        0
    } else {
        (left / gcd(left, right))
            .checked_mul(right)
            .ok_or_else(overflow)?
    };
    i64::try_from(result).map_err(|_| overflow())
}

fn parse_rounded_division(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    let name = parser.word()?;
    require_token(parser, Token::LParen, "rounded division requires '('")?;
    let dividend = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::Comma,
        "rounded division requires two arguments",
    )?;
    let divisor = expression(parser, depth + 1)?;
    require_token(
        parser,
        Token::RParen,
        "expected ')' after rounded division arguments",
    )?;
    if divisor == 0 {
        return Err(ParseError("rounded division by zero".into()));
    }

    // i128 also handles i64::MIN / -1 without overflowing before validation.
    let dividend = i128::from(dividend);
    let divisor = i128::from(divisor);
    let quotient = dividend / divisor;
    let remainder = dividend % divisor;
    let same_sign = (dividend < 0) == (divisor < 0);
    let result = if remainder == 0 {
        quotient
    } else if name.eq_ignore_ascii_case("CEIL_DIV") && same_sign {
        quotient + 1
    } else if name.eq_ignore_ascii_case("FLOOR_DIV") && !same_sign {
        quotient - 1
    } else {
        quotient
    };
    i64::try_from(result).map_err(|_| overflow())
}

fn factor(parser: &mut Parser, depth: usize) -> Result<i64, ParseError> {
    if depth >= MAX_DEPTH {
        return Err(ParseError(
            "scalar expressions nest at most 16 levels".into(),
        ));
    }
    if parser.take(Token::Minus) {
        return factor(parser, depth + 1)?
            .checked_neg()
            .ok_or_else(overflow);
    }
    if parser.take(Token::Plus) {
        return factor(parser, depth + 1);
    }
    if parser.keyword("POWER") || parser.keyword("POW") {
        return parse_power(parser, depth);
    }
    if parser.keyword("BIT_WIDTH") {
        return parse_bit_width(parser, depth);
    }
    if parser.keyword("CLAMP") {
        return parse_clamp(parser, depth);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("CEIL_DIV") || name.eq_ignore_ascii_case("FLOOR_DIV"))
    {
        return parse_rounded_division(parser, depth);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("GCD") || name.eq_ignore_ascii_case("LCM"))
    {
        return parse_gcd_lcm(parser, depth);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if is_extrema(name)) {
        return parse_extrema(parser, depth);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if is_function(name)) {
        let name = parser.word()?;
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' after scalar function".into()));
        }
        let first = expression(parser, depth + 1)?;
        let result = if is_bitwise(&name) {
            if !parser.take(Token::Comma) {
                return Err(ParseError("bitwise functions require two arguments".into()));
            }
            let second = expression(parser, depth + 1)?;
            match name.to_ascii_uppercase().as_str() {
                "BITAND" => first & second,
                "BITOR" => first | second,
                "BITXOR" => first ^ second,
                "BITSHIFTLEFT" => checked_bit_shift(first, second, true)?,
                "BITSHIFTRIGHT" => checked_bit_shift(first, second, false)?,
                "BITROTATELEFT" => checked_bit_rotate(first, second, true)?,
                "BITROTATERIGHT" => checked_bit_rotate(first, second, false)?,
                _ => unreachable!("only supported bitwise functions reach this branch"),
            }
        } else if name.eq_ignore_ascii_case("ABS") {
            first.checked_abs().ok_or_else(overflow)?
        } else {
            first.signum()
        };
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after scalar function".into()));
        }
        return Ok(result);
    }
    if parser.take(Token::LParen) {
        let value = expression(parser, depth + 1)?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' in scalar expression".into()));
        }
        return Ok(value);
    }
    parser
        .number()?
        .parse::<i64>()
        .map_err(|_| ParseError("SELECT literals must be signed 64-bit integers".into()))
}

fn overflow() -> ParseError {
    ParseError("signed 64-bit scalar expression overflow".into())
}
