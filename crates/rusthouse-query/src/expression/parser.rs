use super::*;

fn parse_length_column(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parser.index += 2;
    let column = parser.word()?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("length requires a single column".into()));
    }
    Ok(ScalarExpr::TextLength(column))
}

fn parse_parenthesized(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    let result = parse_expression_tokens(parser)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' in scalar expression".into()));
    }
    Ok(result)
}

pub(super) fn primary(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::Plus) => {
            parser.index += 1;
            primary(parser)
        }
        Some(Token::Minus) => {
            parser.index += 1;
            Ok(ScalarExpr::Subtract(
                Box::new(ScalarExpr::Integer(0)),
                Box::new(primary(parser)?),
            ))
        }
        Some(Token::Word(name))
            if name.eq_ignore_ascii_case("length")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            parse_length_column(parser)
        }
        Some(Token::Word(_)) => Ok(ScalarExpr::Column(parser.word()?)),
        Some(Token::Number(number)) => {
            let literal = if number.contains('.') {
                let value = number.parse::<f64>().map_err(|_| {
                    ParseError("invalid decimal expression literal".into())
                })?;
                if !value.is_finite() {
                    return Err(ParseError("decimal expression literal must be finite".into()));
                }
                ScalarExpr::Decimal(number.clone())
            } else {
                let value = number.parse::<i64>().map_err(|_| {
                    ParseError("integer expression literal is out of Int64 range".into())
                })?;
                ScalarExpr::Integer(value)
            };
            parser.index += 1;
            Ok(literal)
        }
        Some(Token::LParen) => parse_parenthesized(parser),
        _ => Err(ParseError(
            "expected column, integer, or '(' in expression".into(),
        )),
    }
}

pub(super) fn product(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let mut expr = primary(parser)?;
    loop {
        let multiply = if parser.take(Token::Star) {
            true
        } else if parser.take(Token::Slash) {
            false
        } else {
            break;
        };
        let right = Box::new(primary(parser)?);
        expr = if multiply {
            ScalarExpr::Multiply(Box::new(expr), right)
        } else {
            ScalarExpr::Divide(Box::new(expr), right)
        };
    }
    Ok(expr)
}
