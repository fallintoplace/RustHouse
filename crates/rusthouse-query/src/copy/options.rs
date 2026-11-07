use super::*;

pub(super) fn parse_copy_format_option(parser: &mut Parser) -> Result<CopyFormat, ParseError> {
    for (keyword, format) in [
        ("CSV", CopyFormat::Csv),
        ("TSV", CopyFormat::Tsv),
        ("TABSEPARATED", CopyFormat::Tsv),
        ("SSV", CopyFormat::Ssv),
        ("JSONEACHROW", CopyFormat::JsonEachRow),
        ("JSONL", CopyFormat::JsonEachRow),
        ("NDJSON", CopyFormat::JsonEachRow),
    ] {
        if parser.keyword(keyword) {
            return Ok(format);
        }
    }
    Err(ParseError(
        "COPY format must be CSV, TSV, SSV or JSONEachRow".into(),
    ))
}

pub(super) fn parse_copy_header(parser: &mut Parser) -> Result<bool, ParseError> {
    if !parser.keyword("HEADER") {
        return Err(ParseError("expected HEADER after CSV".into()));
    }
    if parser.keyword("TRUE") || parser.keyword("YES") {
        Ok(true)
    } else if parser.keyword("FALSE") || parser.keyword("NO") {
        Ok(false)
    } else if let Some(Token::Number(value)) = parser.tokens.get(parser.index) {
        let enabled = match value.as_str() {
            "1" => true,
            "0" => false,
            _ => return Err(ParseError("HEADER numeric value must be 0 or 1".into())),
        };
        parser.index += 1;
        Ok(enabled)
    } else if parser.tokens.get(parser.index) == Some(&Token::RParen) {
        Ok(true)
    } else {
        Err(ParseError("HEADER must be TRUE, FALSE, 0 or 1".into()))
    }
}

struct CopyOptions {
    format: CopyFormat,
    header: bool,
}

pub(super) fn parse_copy_options(parser: &mut Parser) -> Result<CopyOptions, ParseError> {
    let mut options = CopyOptions {
        format: CopyFormat::Csv,
        header: false,
    };
    if parser.keyword("WITH") {
        if parser.keyword("CSV") {
            options.header = parser.keyword("HEADER");
            return Ok(options);
        }
        if !parser.take(Token::LParen) {
            return Err(ParseError("COPY WITH requires '('".into()));
        }
        if parser.keyword("FORMAT") {
            options.format = parse_copy_format_option(parser)?;
            if parser.take(Token::Comma) {
                options.header = parse_copy_header(parser)?;
            }
        } else {
            options.header = parse_copy_header(parser)?;
            if parser.take(Token::Comma) {
                if !parser.keyword("FORMAT") {
                    return Err(ParseError(
                        "COPY HEADER option requires FORMAT after comma".into(),
                    ));
                }
                options.format = parse_copy_format_option(parser)?;
            }
        }
        if options.format == CopyFormat::JsonEachRow && options.header {
            return Err(ParseError(
                "JSONEachRow cannot use a CSV-style HEADER".into(),
            ));
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after COPY options".into()));
        }
    }
    Ok(options)
}

pub(super) fn parse_copy_path(parser: &mut Parser) -> Result<String, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::String(path)) if !path.is_empty() => {
            parser.index += 1;
            Ok(path.clone())
        }
        _ => Err(ParseError("COPY requires a quoted local file path".into())),
    }
}

