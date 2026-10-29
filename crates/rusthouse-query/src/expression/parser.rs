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

/// Searched CASE is an expression, not a SELECT-level projection special case.
/// All branches are parsed once, then the bound executor evaluates one branch per row.
fn parse_case_when(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    if !parser.keyword("CASE") || !parser.keyword("WHEN") {
        return Err(ParseError("CASE requires WHEN".into()));
    }
    let mut arms = Vec::new();
    loop {
        if arms.len() >= 32 {
            return Err(ParseError("CASE has more than 32 WHEN branches".into()));
        }
        let condition = parse_expression_tokens(parser)?;
        if !parser.keyword("THEN") {
            return Err(ParseError("CASE WHEN requires THEN".into()));
        }
        let value = parse_expression_tokens(parser)?;
        arms.push((condition, value));
        if !parser.keyword("WHEN") {
            break;
        }
    }
    let otherwise = if parser.keyword("ELSE") {
        parse_expression_tokens(parser)?
    } else {
        // Searched CASE without ELSE yields NULL for unmatched rows.
        ScalarExpr::Null
    };
    if !parser.keyword("END") {
        return Err(ParseError("CASE WHEN requires END".into()));
    }
    // Nest from last to first. The first matching WHEN wins and
    // no later branch is evaluated, including expressions that might fail.
    Ok(arms.into_iter().rev().fold(otherwise, |tail, (condition, value)| {
        ScalarExpr::CaseWhen(
            Box::new(condition),
            Box::new(value),
            Box::new(tail),
        )
    }))
}

fn parse_extremum(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let name = parser.word()?;
    let maximum = name.eq_ignore_ascii_case("GREATEST");
    if !parser.take(Token::LParen) {
        return Err(ParseError("greatest/least requires '('".into()));
    }
    let mut values = Vec::new();
    loop {
        if values.len() >= 16 {
            return Err(ParseError("greatest/least supports at most 16 values".into()));
        }
        values.push(parse_expression_tokens(parser)?);
        if !parser.take(Token::Comma) { break; }
    }
    if values.len() < 2 || !parser.take(Token::RParen) {
        return Err(ParseError("greatest/least requires two values and ')'".into()));
    }
    Ok(ScalarExpr::Extremum(values, maximum))
}

fn parse_abs(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    if !parser.take(Token::LParen) {
        return Err(ParseError("abs requires '('".into()));
    }
    let inner = parse_expression_tokens(parser)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("abs requires exactly one argument".into()));
    }
    Ok(ScalarExpr::Abs(Box::new(inner)))
}

fn parse_if(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    if !parser.take(Token::LParen) {
        return Err(ParseError("if requires '('".into()));
    }
    let condition = parse_expression_tokens(parser)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError("if requires three arguments".into()));
    }
    let yes = parse_expression_tokens(parser)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError("if requires three arguments".into()));
    }
    let no = parse_expression_tokens(parser)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("if requires exactly three arguments".into()));
    }
    Ok(ScalarExpr::CaseWhen(
        Box::new(condition), Box::new(yes), Box::new(no),
    ))
}

fn parse_nullif(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    if !parser.take(Token::LParen) {
        return Err(ParseError("nullIf requires '('".into()));
    }
    let left = parse_expression_tokens(parser)?;
    if !parser.take(Token::Comma) {
        return Err(ParseError("nullIf requires a second value".into()));
    }
    let right = parse_expression_tokens(parser)?;
    if !parser.take(Token::RParen) {
        return Err(ParseError("nullIf requires exactly two values".into()));
    }
    Ok(ScalarExpr::NullIf(Box::new(left), Box::new(right)))
}

fn parse_coalesce(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    parser.index += 1;
    if !parser.take(Token::LParen) {
        return Err(ParseError("coalesce requires '('".into()));
    }
    let mut arguments = Vec::new();
    loop {
        if arguments.len() >= 16 {
            return Err(ParseError("coalesce accepts at most 16 values".into()));
        }
        arguments.push(parse_expression_tokens(parser)?);
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if arguments.len() < 2 || !parser.take(Token::RParen) {
        return Err(ParseError("coalesce needs two or more values and ')'".into()));
    }
    Ok(ScalarExpr::Coalesce(arguments))
}


fn parse_aggregate_builtin(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let name = parser.word()?.to_ascii_uppercase();
    if !parser.take(Token::LParen) {
        return Err(ParseError("aggregate function requires '('".into()));
    }
    let mut args = Vec::new();
    if !parser.take(Token::RParen) {
        loop {
            if args.len() == 16 {
                return Err(ParseError(
                    "aggregate scalar function supports at most 16 arguments".into(),
                ));
            }
            args.push(parse_expression_tokens(parser)?);
            if parser.take(Token::RParen) {
                break;
            }
            if !parser.take(Token::Comma) {
                return Err(ParseError(
                    "expected ',' or ')' in aggregate scalar function".into(),
                ));
            }
        }
    }
    match (name.as_str(), args.len()) {
        ("IFNULL", 2) => Ok(ScalarExpr::Coalesce(args)),
        ("NVL", 2) => Ok(ScalarExpr::Coalesce(args)),
        ("ISNULL", 1) => Ok(ScalarExpr::IsNull(Box::new(args.remove(0)), false)),
        _ => Err(ParseError(format!("unsupported aggregate function: {name}"))),
    }
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
        Some(Token::Word(name)) if name.eq_ignore_ascii_case("CASE") => {
            parse_case_when(parser)
        }
        Some(Token::Word(name)) if name.eq_ignore_ascii_case("COALESCE") =>
            parse_coalesce(parser),
        Some(Token::Word(name)) if name.eq_ignore_ascii_case("NULLIF") =>
            parse_nullif(parser),
        Some(Token::Word(name)) if name.eq_ignore_ascii_case("IF") =>
            parse_if(parser),
        Some(Token::Word(name)) if name.eq_ignore_ascii_case("ABS") =>
            parse_abs(parser),
        Some(Token::Word(name)) if name.eq_ignore_ascii_case("GREATEST")
            || name.eq_ignore_ascii_case("LEAST") => parse_extremum(parser),
        Some(Token::Word(name)) if name.eq_ignore_ascii_case("NULL") => {
            parser.index += 1;
            Ok(ScalarExpr::Null)
        }
        Some(Token::Word(_))
            if parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            parse_aggregate_builtin(parser)
        }
        Some(Token::Word(_)) => Ok(ScalarExpr::Column(parser.word()?)),
        Some(Token::Number(number)) => {
            let number = number.parse::<i64>().map_err(|_| {
                ParseError("integer expression literal is out of Int64 range".into())
            })?;
            parser.index += 1;
            Ok(ScalarExpr::Integer(number))
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
        let operator = if parser.take(Token::Star) {
            0
        } else if parser.take(Token::Slash) {
            1
        } else if parser.take(Token::Percent) {
            2
        } else {
            break;
        };
        let right = Box::new(primary(parser)?);
        expr = match operator {
            0 => ScalarExpr::Multiply(Box::new(expr), right),
            1 => ScalarExpr::Divide(Box::new(expr), right),
            _ => ScalarExpr::Modulo(Box::new(expr), right),
        };
    }
    Ok(expr)
}
