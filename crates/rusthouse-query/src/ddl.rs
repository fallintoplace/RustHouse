use super::{ParseError, Parser, Token, tokenize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ddl {
    CreateTable {
        name: String,
        columns: Vec<ColumnDef>,
        if_not_exists: bool,
    },
}

fn identifier(parser: &mut Parser) -> Result<String, ParseError> {
    let value = parser.word()?;
    if value.len() > 128 {
        return Err(ParseError("DDL identifiers cannot exceed 128 bytes".into()));
    }
    Ok(value)
}

fn data_type(parser: &mut Parser, nesting: usize) -> Result<String, ParseError> {
    let name = parser.word()?;
    let canonical = match name.to_ascii_uppercase().as_str() {
        "UINT8" => "UInt8",
        "UINT16" => "UInt16",
        "UINT32" => "UInt32",
        "UINT64" => "UInt64",
        "UINT128" => "UInt128",
        "INT8" => "Int8",
        "INT16" => "Int16",
        "INT32" => "Int32",
        "INT64" => "Int64",
        "INT128" => "Int128",
        "FLOAT32" => "Float32",
        "FLOAT64" => "Float64",
        "STRING" => "String",
        "DATE" => "Date",
        "DATETIME" => "DateTime",
        "BOOL" => "Bool",
        "NULLABLE" => "Nullable",
        "ARRAY" => "Array",
        _ => return Err(ParseError(format!("unsupported DDL column type: {name}"))),
    };
    if canonical == "Nullable" || canonical == "Array" {
        if nesting >= 4 {
            return Err(ParseError("DDL type nesting exceeds 4 levels".into()));
        }
        if !parser.take(Token::LParen) {
            return Err(ParseError(format!("{canonical} requires a type argument")));
        }
        let child = data_type(parser, nesting + 1)?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after column type".into()));
        }
        Ok(format!("{canonical}({child})"))
    } else {
        Ok(canonical.into())
    }
}

/// Recognize only supported schema-only DDL. SELECT is parsed by the existing parser.
pub fn parse_ddl(sql: &str) -> Result<Option<Ddl>, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    if !parser.keyword("CREATE") {
        return Ok(None);
    }
    if !parser.keyword("TABLE") {
        return Err(ParseError("expected TABLE after CREATE".into()));
    }
    let if_not_exists = if parser.keyword("IF") {
        if !parser.keyword("NOT") || !parser.keyword("EXISTS") {
            return Err(ParseError("expected IF NOT EXISTS".into()));
        }
        true
    } else {
        false
    };
    let name = identifier(&mut parser)?;
    if !parser.take(Token::LParen) {
        return Err(ParseError("expected '(' after table name".into()));
    }
    let mut columns = Vec::new();
    let mut names = HashSet::new();
    loop {
        if columns.len() == 64 {
            return Err(ParseError("DDL supports at most 64 columns".into()));
        }
        let column_name = identifier(&mut parser)?;
        if !names.insert(column_name.clone()) {
            return Err(ParseError(format!("duplicate DDL column: {column_name}")));
        }
        let column_type = data_type(&mut parser, 0)?;
        columns.push(ColumnDef {
            name: column_name,
            data_type: column_type,
        });
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if !parser.take(Token::RParen) {
        return Err(ParseError("expected ')' after column definitions".into()));
    }
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unexpected tokens after DDL statement".into()));
    }
    Ok(Some(Ddl::CreateTable {
        name,
        columns,
        if_not_exists,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_create_with_nested_types_and_idempotency() {
        assert_eq!(
            parse_ddl("create table if not exists events (id uint64, payload Nullable(Array(String)));")
                .unwrap(),
            Some(Ddl::CreateTable {
                name: "events".into(),
                if_not_exists: true,
                columns: vec![
                    ColumnDef {
                        name: "id".into(),
                        data_type: "UInt64".into(),
                    },
                    ColumnDef {
                        name: "payload".into(),
                        data_type: "Nullable(Array(String))".into(),
                    },
                ],
            })
        );
        assert_eq!(parse_ddl("SELECT 1").unwrap(), None);
    }

    #[test]
    fn rejects_invalid_create_syntax_and_duplicates() {
        for sql in [
            "CREATE TABLE t ()",
            "CREATE TABLE t (id Unknown)",
            "CREATE TABLE t (id UInt64, id Int32)",
            "CREATE TABLE t (id UInt64,)",
            "CREATE TABLE t (id UInt64); DROP TABLE t",
            "CREATE TABLE IF EXISTS t (id UInt64)",
            "CREATE TABLE t (id UInt64) ENGINE = MergeTree",
            "CREATE TABLE t (id Nullable(UInt64)",
        ] {
            assert!(parse_ddl(sql).is_err(), "{sql}");
        }
    }
}
