//! Deliberately narrow INSERT ... VALUES parser for local writable tables.
use super::{ParseError, Parser, Token};
use crate::lexer::tokenize_insert;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insert {
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub rows: Vec<Vec<Option<String>>>,
}

fn parse_insert_identifier(parser: &mut Parser) -> Result<String, ParseError> {
    let name = parser.word()?;
    if name.len() > 128 {
        return Err(ParseError("identifiers cannot exceed 128 bytes".into()));
    }
    Ok(name)
}

fn value(parser: &mut Parser) -> Result<Option<String>, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::String(value)) | Some(Token::Number(value)) => {
            parser.index += 1;
            Ok(Some(value.clone()))
        }
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
            parser.index += 1;
            Ok(None)
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

fn parse_insert_columns(parser: &mut Parser) -> Result<Option<Vec<String>>, ParseError> {
    if !parser.take(Token::LParen) {
        return Ok(None);
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if result.len() >= 128 {
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
    }
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' after INSERT columns".into()));
    }
    Ok(Some(result))
}

/// Recognize INSERT INTO table [(column, ...)] VALUES (...), (...).
/// Only literal rows are accepted, with no expressions or implicit casts.
pub fn parse_insert(sql: &str) -> Result<Option<Insert>, ParseError> {
    let mut parser = Parser {
        tokens: tokenize_insert(sql)?,
        index: 0,
    };
    if !parser.keyword("INSERT") {
        return Ok(None);
    }
    if !parser.keyword("INTO") {
        return Err(ParseError("expected INTO after INSERT".into()));
    }
    let table = parse_insert_identifier(&mut parser)?;
    let columns = parse_insert_columns(&mut parser)?;
    // DEFAULT VALUES inserts one row with no explicit column values.
    // The database writer validates that every omitted column is nullable.
    if parser.keyword("DEFAULT") {
        if columns.is_some() {
            return Err(ParseError("DEFAULT VALUES cannot use a column list".into()));
        }
        if !parser.keyword("VALUES") {
            return Err(ParseError("expected VALUES after DEFAULT".into()));
        }
        parser.take(Token::Semicolon);
        if parser.index != parser.tokens.len() {
            return Err(ParseError(
                "unexpected tokens after INSERT statement".into(),
            ));
        }
        return Ok(Some(Insert {
            table,
            columns: Some(Vec::new()),
            rows: vec![Vec::new()],
        }));
    }
    if !parser.keyword("VALUES") {
        return Err(ParseError("expected VALUES after INSERT table".into()));
    }
    let mut rows = Vec::new();
    loop {
        if rows.len() >= 8192 {
            return Err(ParseError("INSERT supports at most 8192 rows".into()));
        }
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected '(' after VALUES".into()));
        }
        let mut row = Vec::new();
        loop {
            if row.len() >= 128 {
                return Err(ParseError(
                    "INSERT supports at most 128 values per row".into(),
                ));
            }
            row.push(value(&mut parser)?);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after row values".into()));
        }
        if let Some(names) = &columns {
            if row.len() != names.len() {
                return Err(ParseError(
                    "INSERT row width does not match column list".into(),
                ));
            }
        } else if rows
            .first()
            .is_some_and(|previous: &Vec<Option<String>>| previous.len() != row.len())
        {
            return Err(ParseError("INSERT rows must have equal width".into()));
        }
        rows.push(row);
        if !parser.take(Token::Comma) {
            break;
        }
    }
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected tokens after INSERT statement".into(),
        ));
    }
    Ok(Some(Insert {
        table,
        columns,
        rows,
    }))
}

/// ClickHouse's streaming INSERT format header. Payload is provided over
/// stdin separately so large imports are not buffered or SQL-tokenized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertFormatHeader {
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub format: InsertStreamFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertStreamFormat {
    JsonEachRow,
    Csv,
    CsvWithNames,
    TabSeparated,
    TabSeparatedWithNames,
}

pub fn parse_insert_format_header(sql: &str) -> Result<Option<InsertFormatHeader>, ParseError> {
    let mut parser = Parser {
        tokens: crate::lexer::tokenize(sql)?,
        index: 0,
    };
    if !parser.keyword("INSERT") {
        return Ok(None);
    }
    if !parser.keyword("INTO") {
        return Err(ParseError("expected INTO after INSERT".into()));
    }
    let table = parse_insert_identifier(&mut parser)?;
    let columns = parse_insert_columns(&mut parser)?;
    if !parser.keyword("FORMAT") {
        return Ok(None);
    }
    let format = if parser.keyword("JSONEACHROW") {
        InsertStreamFormat::JsonEachRow
    } else if parser.keyword("CSVWITHNAMES") {
        InsertStreamFormat::CsvWithNames
    } else if parser.keyword("CSV") {
        InsertStreamFormat::Csv
    } else if parser.keyword("TABSEPARATEDWITHNAMES") {
        InsertStreamFormat::TabSeparatedWithNames
    } else if parser.keyword("TABSEPARATED") {
        InsertStreamFormat::TabSeparated
    } else {
        return Err(ParseError("unsupported streaming INSERT format".into()));
    };
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unexpected tokens after INSERT FORMAT".into()));
    }
    Ok(Some(InsertFormatHeader { table, columns, format }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_multirow_insert_and_escaped_literals() {
        assert_eq!(
            parse_insert("INSERT INTO events (name, id) VALUES ('O''Brien', 1), ('two', 2);")
                .unwrap(),
            Some(Insert {
                table: "events".into(),
                columns: Some(vec!["name".into(), "id".into()]),
                rows: vec![
                    vec![Some("O'Brien".into()), Some("1".into())],
                    vec![Some("two".into()), Some("2".into())],
                ],
            })
        );
        assert_eq!(
            parse_insert("insert into events values (-2, null)")
                .unwrap()
                .unwrap()
                .rows,
            vec![vec![Some("-2".into()), None]]
        );
        assert!(parse_insert("SELECT 1").unwrap().is_none());
    }

    #[test]
    fn default_values_is_one_row_without_explicit_columns() {
        assert_eq!(
            parse_insert("INSERT INTO events DEFAULT VALUES;").unwrap(),
            Some(Insert {
                table: "events".into(),
                columns: Some(Vec::new()),
                rows: vec![Vec::new()],
            })
        );
        for sql in [
            "INSERT INTO events DEFAULT",
            "INSERT INTO events DEFAULT VALUES, (1)",
            "INSERT INTO events (id) DEFAULT VALUES",
            "INSERT INTO events DEFAULT VALUES; SELECT 1",
        ] {
            assert!(parse_insert(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_more_than_4096_values_rows_and_rejects_over_8192() {
        let values = vec!["(1)"; 8192].join(", ");
        let sql = format!("INSERT INTO events VALUES {values}");
        assert_eq!(parse_insert(&sql).unwrap().unwrap().rows.len(), 8192);
        let overflow = format!("{sql}, (1)");
        assert!(parse_insert(&overflow).is_err());
    }

    #[test]
    fn rejects_more_than_64_insert_columns() {
        let columns = (0..129)
            .map(|index| format!("c{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        assert!(parse_insert(&format!("INSERT INTO t ({columns}) VALUES (1)")).is_err());
    }

    #[test]
    fn rejects_expressions_duplicate_columns_and_multiple_statements() {
        for input in [
            "INSERT events VALUES (1)",
            "INSERT INTO events VALUES ()",
            "INSERT INTO events VALUES (1), (2, 3)",
            "INSERT INTO events VALUES (1 + 2)",
            "INSERT INTO events VALUES (NOW())",
            "INSERT INTO events VALUES (one)",
            "INSERT INTO events (id, id) VALUES (1, 2)",
            "INSERT INTO events (id, name) VALUES (1)",
            "INSERT INTO events VALUES (1); DELETE FROM events",
        ] {
            assert!(parse_insert(input).is_err(), "{input}");
        }
    }
    #[test]
    fn accepts_large_insert_batches_but_keeps_a_bounded_budget() {
        let text = "x".repeat(5000);
        let sql = format!("INSERT INTO events VALUES ('{text}')");
        assert_eq!(
            parse_insert(&sql).unwrap().unwrap().rows,
            vec![vec![Some(text)]]
        );
        let oversized = format!("INSERT INTO events VALUES ('{}')", "x".repeat(1024 * 1024));
        assert!(parse_insert(&oversized).is_err());
    }

    #[test]
    fn accepts_unary_plus_only_before_insert_numbers() {
        let parsed = parse_insert("INSERT INTO events VALUES (+3, -4)")
            .unwrap()
            .unwrap();
        assert_eq!(parsed.rows, vec![vec![Some("3".into()), Some("-4".into())]]);
        for invalid in [
            "INSERT INTO events VALUES (+NULL)",
            "INSERT INTO events VALUES (+'text')",
        ] {
            assert!(parse_insert(invalid).is_err(), "{invalid}");
        }
    }
}
