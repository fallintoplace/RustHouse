use super::*;

fn parse_length_column(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parser.index += 2;
    let column = parser.word()?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("length requires a single column".into()));
    }
    Ok(ScalarExpr::TextLength(column))
}

fn parse_parenthesized_scalar_expression(
    parser: &mut Parser,
    depth: usize,
) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    let result = parse_expression_tokens_at_depth(parser, depth + 1)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' in scalar expression".into()));
    }
    Ok(result)
}

fn parse_case(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    if !parser.keyword("CASE") {
        return Err(ParseError("CASE expression requires CASE".into()));
    }
    let mut branches = Vec::new();
    while parser.keyword("WHEN") {
        if branches.len() >= 16 {
            return Err(ParseError("CASE supports at most 16 WHEN branches".into()));
        }
        let condition = parse_expression_tokens_at_depth(parser, depth + 1)?;
        if !parser.keyword("THEN") {
            return Err(ParseError("CASE WHEN requires THEN".into()));
        }
        let value = parse_expression_tokens_at_depth(parser, depth + 1)?;
        branches.push((condition, value));
    }
    if branches.is_empty() {
        return Err(ParseError("CASE requires WHEN".into()));
    }
    let else_expr = if parser.keyword("ELSE") {
        Some(Box::new(parse_expression_tokens_at_depth(
            parser,
            depth + 1,
        )?))
    } else {
        None
    };
    if !parser.keyword("END") {
        return Err(ParseError("CASE expression requires END".into()));
    }
    Ok(ScalarExpr::Case {
        branches,
        else_expr,
    })
}

fn parse_numeric_literal(number: &str) -> Result<ScalarExpr, ParseError> {
    if number
        .bytes()
        .any(|byte| matches!(byte, b'.' | b'e' | b'E'))
    {
        let value = number
            .parse::<f64>()
            .map_err(|_| ParseError("invalid decimal expression literal".into()))?;
        if !value.is_finite() {
            return Err(ParseError(
                "decimal expression literal must be finite".into(),
            ));
        }
        Ok(ScalarExpr::Decimal(number.to_owned()))
    } else {
        let value = number
            .parse::<i64>()
            .map_err(|_| ParseError("integer expression literal is out of Int64 range".into()))?;
        Ok(ScalarExpr::Integer(value))
    }
}

pub(super) fn primary(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    if depth >= MAX_EXPRESSION_DEPTH {
        return Err(ParseError(
            "scalar expression exceeds nesting depth of 64".into(),
        ));
    }
    match parser.tokens.get(parser.index) {
        Some(Token::Plus) => {
            parser.index += 1;
            primary(parser, depth + 1)
        }
        Some(Token::Minus) => {
            parser.index += 1;
            // The magnitude of Int64::MIN does not fit in a positive Int64.
            // The lexer already handles the adjacent "-922..." spelling,
            // but whitespace between sign and number yields separate tokens.
            if matches!(
                parser.tokens.get(parser.index),
                Some(Token::Number(number)) if number == "9223372036854775808"
            ) {
                parser.index += 1;
                return Ok(ScalarExpr::Integer(i64::MIN));
            }
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
        Some(Token::Word(name)) if name.eq_ignore_ascii_case("CASE") => parse_case(parser, depth),
        Some(Token::Word(_)) => Ok(ScalarExpr::Column(parser.word()?)),
        Some(Token::Number(number)) => {
            let literal = parse_numeric_literal(number)?;
            parser.index += 1;
            Ok(literal)
        }
        Some(Token::LParen) => parse_parenthesized_scalar_expression(parser, depth),
        _ => Err(ParseError(
            "expected column, integer, or '(' in expression".into(),
        )),
    }
}

pub(super) fn product(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    let mut expr = primary(parser, depth)?;
    let mut operators = 0usize;
    loop {
        let operator = if parser.take(Token::Star) {
            Token::Star
        } else if parser.take(Token::Slash) {
            Token::Slash
        } else if parser.take(Token::Percent) {
            Token::Percent
        } else {
            break;
        };
        operators += 1;
        if operators > 128 {
            return Err(ParseError(
                "multiplicative operator chain exceeds 128 terms".into(),
            ));
        }
        let right = Box::new(primary(parser, depth)?);
        expr = match operator {
            Token::Star => ScalarExpr::Multiply(Box::new(expr), right),
            Token::Slash => ScalarExpr::Divide(Box::new(expr), right),
            Token::Percent => ScalarExpr::Modulo(Box::new(expr), right),
            _ => unreachable!("multiplicative operator"),
        };
    }
    Ok(expr)
}

#[cfg(test)]
mod scientific_expression_tests {
    use super::*;

    #[test]
    fn exponent_only_literals_are_decimal_expressions() {
        for literal in ["1e3", "2E-2", "0e+5", "-3e1"] {
            assert_eq!(
                super::super::parse_scalar_expression(literal).unwrap(),
                ScalarExpr::Decimal(literal.to_owned()),
                "{literal}"
            );
        }
        assert!(super::super::parse_scalar_expression("1e309").is_err());
        assert!(super::super::parse_scalar_expression("1e").is_err());
    }
}

#[cfg(test)]
mod modulo_expression_tests {
    use super::*;

    #[test]
    fn modulo_uses_multiplicative_precedence() {
        let expression = super::super::parse_scalar_expression("amount % 5 * 2 + 1").unwrap();
        assert_eq!(
            expression,
            ScalarExpr::Add(
                Box::new(ScalarExpr::Multiply(
                    Box::new(ScalarExpr::Modulo(
                        Box::new(ScalarExpr::Column("amount".into())),
                        Box::new(ScalarExpr::Integer(5)),
                    )),
                    Box::new(ScalarExpr::Integer(2)),
                )),
                Box::new(ScalarExpr::Integer(1)),
            )
        );
        assert!(super::super::parse_scalar_expression("amount %").is_err());
    }
}

#[cfg(test)]
mod separated_min_int64_literal_tests {
    use super::*;

    #[test]
    fn whitespace_before_minimum_signed_integer_preserves_exact_value() {
        for source in [
            "-9223372036854775808",
            "- 9223372036854775808",
            "- 9_223_372_036_854_775_808",
            "-\t9223372036854775808",
        ] {
            assert_eq!(
                super::super::parse_scalar_expression(source).unwrap(),
                ScalarExpr::Integer(i64::MIN),
                "{source}"
            );
        }
    }

    #[test]
    fn rejects_unrepresentable_positive_or_more_negative_literals() {
        for source in [
            "9223372036854775808",
            "+ 9223372036854775808",
            "- 9223372036854775809",
            "- 18446744073709551616",
        ] {
            assert!(
                super::super::parse_scalar_expression(source).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn normal_separated_unary_minus_keeps_existing_ast() {
        assert_eq!(
            super::super::parse_scalar_expression("- 17").unwrap(),
            ScalarExpr::Subtract(
                Box::new(ScalarExpr::Integer(0)),
                Box::new(ScalarExpr::Integer(17)),
            )
        );
    }
}
