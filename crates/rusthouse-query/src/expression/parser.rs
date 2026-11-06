use super::*;

fn parse_length_column(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parser.index += 2;
    let column = parser.word()?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("length requires a single column".into()));
    }
    Ok(ScalarExpr::TextLength(column))
}

fn parse_concat(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    if !parser.take(Token::LParen) {
        return Err(ParseError("CONCAT requires '('".into()));
    }
    let mut arguments = Vec::new();
    loop {
        if arguments.len() == 16 {
            return Err(ParseError("CONCAT supports at most 16 arguments".into()));
        }
        arguments.push(parse_expression_tokens_at_depth(parser, depth + 1)?);
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if arguments.len() < 2 || !parser.take(Token::RParen) {
        return Err(ParseError(
            "CONCAT requires at least two arguments and ')'".into(),
        ));
    }
    Ok(ScalarExpr::TextConcat(arguments))
}

fn parse_substring(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    if !parser.take(Token::LParen) {
        return Err(ParseError("SUBSTRING requires '('".into()));
    }
    let value = Box::new(parse_expression_tokens_at_depth(parser, depth + 1)?);
    if !parser.take(Token::Comma) {
        return Err(ParseError("SUBSTRING requires a start position".into()));
    }
    let start = parser
        .number()?
        .parse::<usize>()
        .ok()
        .filter(|value| (1..=1_000_000_000).contains(value))
        .ok_or_else(|| ParseError("SUBSTRING start must be a positive integer".into()))?;
    let length = if parser.take(Token::Comma) {
        Some(
            parser
                .number()?
                .parse::<usize>()
                .ok()
                .filter(|value| *value <= 1_000_000)
                .ok_or_else(|| ParseError("SUBSTRING length must be a bounded integer".into()))?,
        )
    } else {
        None
    };
    if !parser.take(Token::RParen) {
        return Err(ParseError("SUBSTRING requires ')'".into()));
    }
    Ok(ScalarExpr::TextSubstring {
        value,
        start,
        length,
    })
}

fn parse_round(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    if !parser.take(Token::LParen) {
        return Err(ParseError("ROUND requires '('".into()));
    }
    let value = Box::new(parse_expression_tokens_at_depth(parser, depth + 1)?);
    let precision = if parser.take(Token::Comma) {
        let has_minus_token = parser.take(Token::Minus);
        let number = parser.number()?;
        let (negative, digits) = if let Some(digits) = number.strip_prefix('-') {
            if has_minus_token {
                return Err(ParseError("ROUND precision is invalid".into()));
            }
            (true, digits)
        } else {
            (has_minus_token, number.as_str())
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError("ROUND precision requires an integer".into()));
        }
        let precision = digits
            .parse::<i32>()
            .map_err(|_| ParseError("ROUND precision is out of range".into()))?;
        if precision > 308 {
            return Err(ParseError("ROUND precision is out of range".into()));
        }
        if negative { -precision } else { precision }
    } else {
        0
    };
    if !parser.take(Token::RParen) {
        return Err(ParseError("ROUND requires ')'".into()));
    }
    Ok(ScalarExpr::Round { value, precision })
}

fn parse_coalesce(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    if !parser.take(Token::LParen) {
        return Err(ParseError("COALESCE requires '('".into()));
    }
    let mut arguments = Vec::new();
    loop {
        if arguments.len() == 16 {
            return Err(ParseError("COALESCE supports at most 16 arguments".into()));
        }
        arguments.push(parse_expression_tokens_at_depth(parser, depth + 1)?);
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if arguments.len() < 2 || !parser.take(Token::RParen) {
        return Err(ParseError(
            "COALESCE requires at least two arguments and ')'".into(),
        ));
    }
    let mut result = arguments
        .pop()
        .expect("COALESCE has at least two arguments");
    while let Some(argument) = arguments.pop() {
        let ScalarExpr::Column(column) = &argument else {
            return Err(ParseError(
                "COALESCE currently requires column arguments before the final value".into(),
            ));
        };
        result = ScalarExpr::Case {
            branches: vec![(
                ScalarExpr::IsNull {
                    column: column.clone(),
                    negated: true,
                },
                argument,
            )],
            else_expr: Some(Box::new(result)),
        };
    }
    Ok(result)
}

fn parse_date_cast(parser: &mut Parser, depth: usize) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    if !parser.take(Token::LParen) {
        return Err(ParseError("CAST requires '('".into()));
    }
    let value = match parser.tokens.get(parser.index) {
        Some(Token::String(value)) => {
            let days = rusthouse_storage::parse_date32(value)
                .ok_or_else(|| ParseError("CAST requires a valid DATE literal".into()))?;
            parser.index += 1;
            ScalarExpr::Integer(i64::from(days))
        }
        _ => parse_expression_tokens_at_depth(parser, depth + 1)?,
    };
    if !parser.keyword("AS") {
        return Err(ParseError("CAST requires AS DATE".into()));
    }
    let data_type = parser.word()?;
    if !matches!(data_type.to_ascii_uppercase().as_str(), "DATE" | "DATE32") {
        return Err(ParseError(
            "scalar CAST currently supports DATE only".into(),
        ));
    }
    if !parser.take(Token::RParen) {
        return Err(ParseError("CAST requires closing ')'".into()));
    }
    if !matches!(value, ScalarExpr::Column(_) | ScalarExpr::Integer(_)) {
        return Err(ParseError(
            "CAST AS DATE requires a date column or literal".into(),
        ));
    }
    Ok(value)
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
                return Err(ParseError(
                    "simple CASE NULL matches are unsupported".into(),
                ));
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
                ScalarExpr::Compare(Box::new(operand.clone()), Operator::Eq, Box::new(value))
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
        if parser.keyword("NULL") {
            None
        } else {
            Some(Box::new(parse_expression_tokens_at_depth(
                parser,
                depth + 1,
            )?))
        }
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
        Some(Token::Word(name))
            if name.eq_ignore_ascii_case("concat")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            parse_concat(parser, depth)
        }
        Some(Token::Word(name))
            if name.eq_ignore_ascii_case("substring")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            parse_substring(parser, depth)
        }
        Some(Token::Word(name))
            if name.eq_ignore_ascii_case("coalesce")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            parse_coalesce(parser, depth)
        }
        Some(Token::Word(name))
            if name.eq_ignore_ascii_case("round")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            parse_round(parser, depth)
        }
        Some(Token::Word(name))
            if name.eq_ignore_ascii_case("CAST")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            parse_date_cast(parser, depth)
        }
        Some(Token::Word(name)) if name.eq_ignore_ascii_case("CASE") => parse_case(parser, depth),
        Some(Token::Word(_) | Token::QuotedWord(_)) => Ok(ScalarExpr::Column(parser.word()?)),
        Some(Token::String(value)) => {
            let value = value.clone();
            parser.index += 1;
            Ok(ScalarExpr::TextLiteral(value))
        }
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
mod date_cast_expression_tests {
    use super::*;

    #[test]
    fn parses_date_casts_as_date32_values() {
        assert_eq!(
            super::super::parse_scalar_expression("CAST(d_date AS DATE)").unwrap(),
            ScalarExpr::Column("d_date".into())
        );
        assert_eq!(
            super::super::parse_scalar_expression("CAST('2000-03-11' AS DATE)").unwrap(),
            ScalarExpr::Integer(i64::from(
                rusthouse_storage::parse_date32("2000-03-11").unwrap()
            ))
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
        let expr =
            parse_scalar_expression("CASE country WHEN 'DE' THEN 1 WHEN 'FR' THEN 2 ELSE 0 END")
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

#[cfg(test)]
mod text_concat_expression_tests {
    use crate::expression::{ScalarExpr, parse_scalar_expression};

    #[test]
    fn parses_nested_concat_with_coalesced_string_columns() {
        let expression = parse_scalar_expression(
            "concat(concat(coalesce(last_name, ''), ', '), coalesce(first_name, ''))",
        )
        .unwrap();
        let mut columns = Vec::new();
        expression.referenced_columns(&mut columns);

        assert!(matches!(expression, ScalarExpr::TextConcat(arguments) if arguments.len() == 2));
        assert_eq!(columns, ["last_name", "first_name"]);
    }

    #[test]
    fn parses_round_with_optional_precision() {
        assert_eq!(
            parse_scalar_expression("round((amount / 3), 2)").unwrap(),
            ScalarExpr::Round {
                value: Box::new(ScalarExpr::Divide(
                    Box::new(ScalarExpr::Column("amount".into())),
                    Box::new(ScalarExpr::Integer(3)),
                )),
                precision: 2,
            }
        );
        assert_eq!(
            parse_scalar_expression("round(amount)").unwrap(),
            ScalarExpr::Round {
                value: Box::new(ScalarExpr::Column("amount".into())),
                precision: 0,
            }
        );
        assert!(parse_scalar_expression("round(amount, 1.5)").is_err());
    }

    #[test]
    fn rejects_concat_with_too_few_or_too_many_arguments() {
        assert!(parse_scalar_expression("concat(last_name)").is_err());
        let arguments = std::iter::repeat_n("last_name", 17)
            .collect::<Vec<_>>()
            .join(", ");
        assert!(parse_scalar_expression(&format!("concat({arguments})")).is_err());
    }

    #[test]
    fn parses_bounded_substring_positions_and_lengths() {
        assert_eq!(
            parse_scalar_expression("substring(city, 2, 5)").unwrap(),
            ScalarExpr::TextSubstring {
                value: Box::new(ScalarExpr::Column("city".into())),
                start: 2,
                length: Some(5),
            }
        );
        assert!(matches!(
            parse_scalar_expression("substring(city, 3)").unwrap(),
            ScalarExpr::TextSubstring {
                start: 3,
                length: None,
                ..
            }
        ));
        for input in ["substring(city, 0, 1)", "substring(city, 1, 1000001)"] {
            assert!(parse_scalar_expression(input).is_err(), "{input}");
        }
    }
}

#[cfg(test)]
mod null_expression_tests {
    use super::*;

    #[test]
    fn parses_null_checks_inside_boolean_conditions() {
        let expression = super::super::parse_scalar_expression(
            "store_customer_sk IS NOT NULL AND catalog_customer_sk IS NULL",
        )
        .unwrap();
        assert_eq!(
            expression,
            ScalarExpr::And(
                Box::new(ScalarExpr::IsNull {
                    column: "store_customer_sk".into(),
                    negated: true,
                }),
                Box::new(ScalarExpr::IsNull {
                    column: "catalog_customer_sk".into(),
                    negated: false,
                }),
            )
        );
    }
}
