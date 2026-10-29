use super::*;

fn parse_length_column(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parser.index += 2;
    let column = parser.word()?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("length requires a single column".into()));
    }
    Ok(ScalarExpr::TextLength(column))
}

fn parse_parenthesized(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    let result = parse_expression_tokens_at_depth(parser, depth + 1)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' in scalar expression".into()));
    }
    Ok(result)
}

pub(super) fn primary(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    if depth >= MAX_EXPRESSION_DEPTH {
        return Err(ParseError("scalar expression exceeds nesting depth of 64".into()));
    }
    match parser.tokens.get(parser.index) {
        Some(Token::Plus) => {
            parser.index += 1;
            primary(parser, depth + 1)
        }
        Some(Token::Minus) => {
            parser.index += 1;
            Ok(ScalarExpr::Subtract(
                Box::new(ScalarExpr::Integer(0)),
                Box::new(primary(parser, depth + 1)?),
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
            let number = number.parse::<i64>().map_err(|_| {
                ParseError("integer expression literal is out of Int64 range".into())
            })?;
            parser.index += 1;
            Ok(ScalarExpr::Integer(number))
        }
        Some(Token::LParen) => parse_parenthesized(parser, depth),
        _ => Err(ParseError(
            "expected column, integer, or '(' in expression".into(),
        )),
    }
}

pub(super) fn product(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    let mut expr = primary(parser, depth)?;
    let mut operators = 0usize;
    loop {
        let multiply = if parser.take(Token::Star) {
            true
        } else if parser.take(Token::Slash) {
            false
        } else {
            break;
        };
        operators += 1;
        if operators > 128 {
            return Err(ParseError("multiplicative operator chain exceeds 128 terms".into()));
        }
        let right = Box::new(primary(parser, depth)?);
        expr = if multiply {
            ScalarExpr::Multiply(Box::new(expr), right)
        } else {
            ScalarExpr::Divide(Box::new(expr), right)
        };
    }
    Ok(expr)
}
