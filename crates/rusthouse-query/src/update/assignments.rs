use super::*;

pub(super) fn parse_update_assignment_value(
    parser: &mut Parser,
    column: &str,
) -> Result<UpdateValue, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::String(value)) | Some(Token::Number(value)) => {
            parser.index += 1;
            Ok(UpdateValue::Literal(Some(value.clone())))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
            parser.index += 1;
            Ok(UpdateValue::Literal(None))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("TRUE") => {
            parser.index += 1;
            Ok(UpdateValue::Literal(Some("1".into())))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("FALSE") => {
            parser.index += 1;
            Ok(UpdateValue::Literal(Some("0".into())))
        }
        Some(Token::Word(word)) if word == column => {
            parser.index += 1;
            let subtract = if parser.take(Token::Plus) {
                false
            } else if parser.take(Token::Minus) {
                true
            } else {
                return Err(ParseError("UPDATE self-reference requires + or -".into()));
            };
            let value = parser.number()?;
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ParseError(
                    "UPDATE offset must be a non-negative integer".into(),
                ));
            }
            let delta = value
                .parse::<u128>()
                .map_err(|_| ParseError("UPDATE offset exceeds UInt128".into()))?;
            Ok(UpdateValue::Offset { delta, subtract })
        }
        Some(Token::Plus) => {
            parser.index += 1;
            match parser.tokens.get(parser.index) {
                Some(Token::Number(value)) if !value.starts_with('-') => {
                    let value = value.clone();
                    parser.index += 1;
                    Ok(UpdateValue::Literal(Some(value)))
                }
                _ => Err(ParseError("UPDATE + requires a numeric literal".into())),
            }
        }
        Some(Token::Minus) => {
            parser.index += 1;
            match parser.tokens.get(parser.index) {
                Some(Token::Number(value)) if !value.starts_with('-') => {
                    let value = format!("-{value}");
                    parser.index += 1;
                    Ok(UpdateValue::Literal(Some(value)))
                }
                _ => Err(ParseError("UPDATE - requires a numeric literal".into())),
            }
        }
        _ => Err(ParseError(
            "UPDATE SET accepts literals, NULL, or checked same-column integer offsets".into(),
        )),
    }
}

pub(super) fn parse_assignments(parser: &mut Parser) -> Result<Vec<(String, UpdateValue)>, ParseError> {
    let mut assignments = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if assignments.len() >= MAX_UPDATE_ASSIGNMENTS {
            return Err(ParseError("UPDATE supports at most 64 columns".into()));
        }
        let column = parser.bounded_word()?;
        if !seen.insert(column.clone()) {
            return Err(ParseError(format!("duplicate UPDATE column: {column}")));
        }
        if !parser.take(Token::Operator(Operator::Eq)) {
            return Err(ParseError("UPDATE assignments require =".into()));
        }
        let assignment = parse_update_assignment_value(parser, &column)?;
        assignments.push((column, assignment));
        if !parser.take(Token::Comma) {
            break;
        }
    }
    Ok(assignments)
}
