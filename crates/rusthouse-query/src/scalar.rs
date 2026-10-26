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

pub(super) fn is_function(name: &str) -> bool {
    name.eq_ignore_ascii_case("ABS") || name.eq_ignore_ascii_case("SIGN") || is_extrema(name) || is_power(name)
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
        if !parser.take(Token::LParen) {
            return Err(ParseError("POWER requires '('".into()));
        }
        let base = expression(parser, depth + 1)?;
        if !parser.take(Token::Comma) {
            return Err(ParseError("POWER requires two arguments".into()));
        }
        let exponent = expression(parser, depth + 1)?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("POWER requires ')' after exponent".into()));
        }
        let exponent = u32::try_from(exponent)
            .map_err(|_| ParseError("POWER exponent must be non-negative and fit UInt32".into()))?;
        return base.checked_pow(exponent).ok_or_else(overflow);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if is_extrema(name)) {
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
        return Ok(value);
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if is_function(name)) {
        let name = parser.word()?;
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' after scalar function".into()));
        }
        let value = expression(parser, depth + 1)?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after scalar function".into()));
        }
        return if name.eq_ignore_ascii_case("ABS") {
            value.checked_abs().ok_or_else(overflow)
        } else {
            Ok(value.signum())
        };
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
