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
    let operand = if matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("WHEN")
    ) {
        None
    } else {
        Some(parse_expression_tokens_at_depth(parser, depth + 1)?)
    };
    let mut branches = Vec::new();
    while parser.keyword("WHEN") {
        if branches.len() >= 16 {
            return Err(ParseError("CASE supports at most 16 WHEN branches".into()));
        }
        let condition = if let Some(operand) = &operand {
            if matches!(
                parser.tokens.get(parser.index),
                Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL")
            ) {
                return Err(ParseError("simple CASE NULL matches are unsupported".into()));
            }
            if let Some(Token::String(value)) = parser.tokens.get(parser.index) {
                let ScalarExpr::Column(column) = operand else {
                    return Err(ParseError(
                        "simple CASE text matches require a column".into(),
                    ));
                };
                let value = value.clone();
                parser.index += 1;
                ScalarExpr::TextCompare {
                    column: column.clone(),
                    operator: Operator::Eq,
                    value,
                }
            } else {
                let value = parse_expression_tokens_at_depth(parser, depth + 1)?;
                ScalarExpr::Compare(
                    Box::new(operand.clone()),
                    Operator::Eq,
                    Box::new(value),
                )
            }
        } else {
            parse_expression_tokens_at_depth(parser, depth + 1)?
        };
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

#[cfg(test)]
mod simple_case_expression_tests {
    use crate::expression::{ScalarExpr, parse_scalar_expression};

    #[test]
    fn numeric_simple_case_matches_existing_searched_case_ast() {
        for (simple, searched) in [
            (
                "CASE amount WHEN 1 THEN 10 WHEN 2 THEN 20 ELSE 0 END",
                "CASE WHEN amount = 1 THEN 10 WHEN amount = 2 THEN 20 ELSE 0 END",
            ),
            (
                "CASE amount % 2 WHEN 0 THEN 1 ELSE 0 END",
                "CASE WHEN amount % 2 = 0 THEN 1 ELSE 0 END",
            ),
        ] {
            assert_eq!(
                parse_scalar_expression(simple).unwrap(),
                parse_scalar_expression(searched).unwrap()
            );
        }
    }

    #[test]
    fn text_simple_case_uses_existing_string_equality_predicate() {
        let expr = parse_scalar_expression(
            "CASE country WHEN 'DE' THEN 1 WHEN 'FR' THEN 2 ELSE 0 END"
        )
        .unwrap();
        let ScalarExpr::Case { branches, .. } = expr else {
            panic!("expected CASE");
        };
        assert!(matches!(
            &branches[0].0,
            ScalarExpr::TextCompare {
                column,
                value,
                ..
            } if column == "country" && value == "DE"
        ));
    }

    #[test]
    fn malformed_simple_case_forms_are_rejected() {
        for input in [
            "CASE amount WHEN 1 10 END",
            "CASE amount ELSE 0 END",
            "CASE amount WHEN NULL THEN 1 END",
            "CASE amount WHEN 'x' THEN 1 END extra",
        ] {
            assert!(parse_scalar_expression(input).is_err(), "{input}");
        }
    }
}
