use super::*;

pub(super) fn parse_insert_identifier(parser: &mut Parser) -> Result<String, ParseError> {
    let name = parser.word()?;
    if name.len() > MAX_IDENTIFIER_BYTES {
        return Err(ParseError("identifiers cannot exceed 128 bytes".into()));
    }
    Ok(name)
}

pub(super) fn value(parser: &mut Parser) -> Result<Option<String>, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::String(value)) | Some(Token::Number(value)) => {
            parser.index += 1;
            Ok(Some(value.clone()))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
            parser.index += 1;
            Ok(None)
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("TRUE") => {
            parser.index += 1;
            Ok(Some("1".into()))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("FALSE") => {
            parser.index += 1;
            Ok(Some("0".into()))
        }
        Some(Token::Plus) => {
            parser.index += 1;
            match parser.tokens.get(parser.index) {
                Some(Token::Number(value)) if !value.starts_with('-') => {
                    let value = value.clone();
                    parser.index += 1;
                    Ok(Some(value))
                }
                _ => Err(ParseError("INSERT + requires a numeric literal".into())),
            }
        }
        _ => Err(ParseError("INSERT values must be literals or NULL".into())),
    }
}

pub(super) fn parse_insert_columns(parser: &mut Parser) -> Result<Option<Vec<String>>, ParseError> {
    if !parser.take(Token::LParen) {
        return Ok(None);
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if result.len() >= MAX_INSERT_COLUMNS {
            return Err(ParseError("INSERT supports at most 128 columns".into()));
        }
        let column = parse_insert_identifier(parser)?;
        if !seen.insert(column.clone()) {
            return Err(ParseError(format!("duplicate INSERT column: {column}")));
        }
        result.push(column);
        if !parser.take(Token::Comma) {
            break;
        }
        if parser.tokens.get(parser.index) == Some(&Token::RParen) {
            break;
        }
    }
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' after INSERT columns".into()));
    }
    Ok(Some(result))
}

pub(super) fn finish_insert(parser: &mut Parser) -> Result<(), ParseError> {
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected tokens after INSERT statement".into(),
        ));
    }
    Ok(())
}

/// Recognize INSERT INTO table [(column, ...)] VALUES (...), (...).
/// Only literal rows are accepted, with no expressions or implicit casts.
