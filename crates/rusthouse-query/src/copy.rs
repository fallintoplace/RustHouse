//! Restricted COPY FROM CSV for local persistent tables.
use super::{ParseError, Parser, Token, tokenize};
use std::collections::HashSet;

const MAX_COPY_COLUMNS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFormat {
    Csv,
    Tsv,
    Ssv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFrom {
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub path: String,
    pub header: bool,
    pub format: CopyFormat,
}

fn parse_copy_column_list(parser: &mut Parser) -> Result<Option<Vec<String>>, ParseError> {
    if !parser.take(Token::LParen) {
        return Ok(None);
    }
    let mut columns = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if columns.len() >= MAX_COPY_COLUMNS {
            return Err(ParseError("COPY supports at most 128 columns".into()));
        }
        let column = parser.bounded_word()?;
        if !seen.insert(column.clone()) {
            return Err(ParseError(format!("duplicate COPY column: {column}")));
        }
        columns.push(column);
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' after COPY columns".into()));
    }
    Ok(Some(columns))
}

fn parse_copy_format_option(parser: &mut Parser) -> Result<CopyFormat, ParseError> {
    if parser.keyword("CSV") {
        Ok(CopyFormat::Csv)
    } else if parser.keyword("TSV") {
        Ok(CopyFormat::Tsv)
    } else if parser.keyword("SSV") {
        Ok(CopyFormat::Ssv)
    } else {
        Err(ParseError("COPY format must be CSV, TSV or SSV".into()))
    }
}

fn parse_copy_header(parser: &mut Parser) -> Result<bool, ParseError> {
    if !parser.keyword("HEADER") {
        return Err(ParseError("expected HEADER after CSV".into()));
    }
    if parser.keyword("TRUE") {
        Ok(true)
    } else if parser.keyword("FALSE") {
        Ok(false)
    } else if parser.tokens.get(parser.index) == Some(&Token::RParen) {
        Ok(true)
    } else {
        Err(ParseError("HEADER must be TRUE or FALSE".into()))
    }
}

struct CopyOptions {
    format: CopyFormat,
    header: bool,
}

fn parse_copy_options(parser: &mut Parser) -> Result<CopyOptions, ParseError> {
    let mut options = CopyOptions {
        format: CopyFormat::Csv,
        header: false,
    };
    if parser.keyword("WITH") {
        if !parser.take(Token::LParen) || !parser.keyword("FORMAT") {
            return Err(ParseError(
                "COPY supports WITH (FORMAT CSV|TSV|SSV[, HEADER TRUE])".into(),
            ));
        }
        options.format = parse_copy_format_option(parser)?;
        if parser.take(Token::Comma) {
            options.header = parse_copy_header(parser)?;
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after COPY options".into()));
        }
    }
    Ok(options)
}

fn parse_copy_path(parser: &mut Parser) -> Result<String, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::String(path)) if !path.is_empty() => {
            parser.index += 1;
            Ok(path.clone())
        }
        _ => Err(ParseError("COPY requires a quoted local file path".into())),
    }
}

pub fn parse_copy_from(sql: &str) -> Result<Option<CopyFrom>, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    if !parser.keyword("COPY") {
        return Ok(None);
    }
    let table = parser.bounded_word()?;
    let columns = parse_copy_column_list(&mut parser)?;
    if !parser.keyword("FROM") {
        return Err(ParseError("COPY requires FROM 'path'".into()));
    }
    let path = parse_copy_path(&mut parser)?;
    let options = parse_copy_options(&mut parser)?;
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unexpected tokens after COPY statement".into()));
    }
    Ok(Some(CopyFrom {
        table,
        columns,
        path,
        header: options.header,
        format: options.format,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_copy_and_preserves_quoted_path() {
        let parsed = parse_copy_from(
            "COPY events (name, id) FROM '/tmp/data''s.csv' WITH (FORMAT CSV, HEADER TRUE);",
        )
        .unwrap()
        .unwrap();
        assert_eq!(parsed.table, "events");
        assert_eq!(parsed.path, "/tmp/data's.csv");
        assert_eq!(parsed.columns, Some(vec!["name".into(), "id".into()]));
        assert!(parsed.header);
        assert!(!parse_copy_from("SELECT 1").unwrap().is_some());
    }

    #[test]
    fn parses_tsv_options_and_rejects_other_formats() {
        let copy = parse_copy_from("COPY events FROM 'events.tsv' WITH (FORMAT TSV, HEADER TRUE)")
            .unwrap()
            .unwrap();
        assert_eq!(copy.format, CopyFormat::Tsv);
        assert!(copy.header);
        let semicolon = parse_copy_from("COPY readings FROM 'measurements.txt' WITH (FORMAT SSV)")
            .unwrap()
            .unwrap();
        assert_eq!(semicolon.format, CopyFormat::Ssv);
        assert!(!semicolon.header);
        assert_eq!(
            parse_copy_from("COPY events FROM 'events.tsv' WITH (FORMAT TSV, HEADER FALSE)")
                .unwrap()
                .unwrap()
                .format,
            CopyFormat::Tsv
        );
        assert_eq!(
            parse_copy_from("COPY events FROM 'events.csv'")
                .unwrap()
                .unwrap()
                .format,
            CopyFormat::Csv
        );
        assert!(parse_copy_from("COPY events FROM 'events' WITH (FORMAT JSON)").is_err());
    }

    #[test]
    fn defaults_header_to_false_for_format_only_options() {
        let copy = parse_copy_from("COPY events FROM 'rows.tsv' WITH (FORMAT TSV)")
            .unwrap()
            .unwrap();
        assert_eq!(copy.format, CopyFormat::Tsv);
        assert!(!copy.header);
    }

    #[test]
    fn enforces_copy_column_limit() {
        let columns = (0..129)
            .map(|index| format!("c{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("COPY t ({columns}) FROM 'rows.csv'");
        assert!(parse_copy_from(&sql).is_err());
    }

    #[test]
    fn refuses_unbounded_or_unsupported_copy_syntax() {
        for sql in [
            "COPY events FROM STDIN",
            "COPY events FROM '/tmp/a.csv' WITH (FORMAT JSON)",
            "COPY events FROM '/tmp/a.csv'; DROP TABLE events",
            "COPY events (id, id) FROM '/tmp/a.csv'",
            "COPY events FROM '/tmp/a.csv' WITH (FORMAT CSV, HEADER MAYBE)",
            "COPY events TO '/tmp/a.csv'",
        ] {
            assert!(parse_copy_from(sql).is_err(), "{sql}");
        }
    }
    #[test]
    fn accepts_copy_header_shorthand() {
        let parsed = parse_copy_from("COPY events FROM 'events.csv' WITH (FORMAT CSV, HEADER)")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.format, CopyFormat::Csv);
        assert!(parsed.header);
        assert!(
            parse_copy_from("COPY events FROM 'x.csv' WITH (FORMAT CSV, HEADER, FORMAT TSV)")
                .is_err()
        );
    }
}
