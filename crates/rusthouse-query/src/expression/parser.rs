use super::*;

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
            parser.index += 2;
            let column = parser.word()?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("length requires a single column".into()));
            }
            Ok(ScalarExpr::TextLength(column))
        }
        Some(Token::Word(_)) => Ok(ScalarExpr::Column(parser.word()?)),
        Some(Token::Number(number)) => {
            let number = number.parse::<i64>().map_err(|_| {
                ParseError("integer expression literal is out of Int64 range".into())
            })?;
            parser.index += 1;
            Ok(ScalarExpr::Integer(number))
        }
        Some(Token::LParen) => {
            parser.index += 1;
            let result = parse_expression_tokens(parser)?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' in scalar expression".into()));
            }
            Ok(result)
        }
        _ => Err(ParseError(
            "expected column, integer, or '(' in expression".into(),
        )),
    }
}

pub(super) fn product(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let mut expr = primary(parser)?;
    loop {
        if parser.take(Token::Star) {
            expr = ScalarExpr::Multiply(Box::new(expr), Box::new(primary(parser)?));
        } else if parser.take(Token::Slash) {
            expr = ScalarExpr::Divide(Box::new(expr), Box::new(primary(parser)?));
        } else {
            break;
        }
    }
    Ok(expr)
}
