//! Bounded constant-expression evaluation for scalar SELECT without FROM.
//! Calculation happens in the parser: the executor still sees a single Literal.
use super::{ParseError, Parser, Token};

const MAX_DEPTH: usize = 16;

pub(super) fn is_function(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "ABS" | "SIGN" | "BITAND" | "BITOR" | "BITXOR"
    )
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
    if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if is_function(name)) {
        let name = parser.word()?;
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' after scalar function".into()));
        }
        let first = expression(parser, depth + 1)?;
        let result = match name.to_ascii_uppercase().as_str() {
            "ABS" => first.checked_abs().ok_or_else(overflow)?,
            "SIGN" => first.signum(),
            "BITAND" | "BITOR" | "BITXOR" => {
                if !parser.take(Token::Comma) {
                    return Err(ParseError("bitwise functions require two arguments".into()));
                }
                let second = expression(parser, depth + 1)?;
                match name.to_ascii_uppercase().as_str() {
                    "BITAND" => first & second,
                    "BITOR" => first | second,
                    _ => first ^ second,
                }
            }
            _ => unreachable!("only supported scalar functions reach this branch"),
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
