use super::*;

pub(super) fn identifier(parser: &mut Parser) -> Result<String, ParseError> {
    let value = parser.word()?;
    if value.len() > 128 {
        return Err(ParseError("DDL identifiers cannot exceed 128 bytes".into()));
    }
    Ok(value)
}

fn canonical_type(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_uppercase().as_str() {
        "TINYINT" => "Int8",
        "SMALLINT" => "Int16",
        "INTEGER" | "INT" => "Int32",
        "BIGINT" => "Int64",
        "TEXT" | "VARCHAR" | "CHARACTER" => "String",
        "TIMESTAMP" => "DateTime",
        "REAL" | "FLOAT" => "Float32",
        "DOUBLE" => "Float64",
        "BOOLEAN" => "Bool",
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
        _ => return None,
    })
}

pub(super) fn data_type(parser: &mut Parser, nesting: usize) -> Result<String, ParseError> {
    let name = parser.word()?;
    let canonical = canonical_type(&name)
        .ok_or_else(|| ParseError(format!("unsupported DDL column type: {name}")))?;
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

pub(super) fn finish_ddl(parser: &mut Parser) -> Result<(), ParseError> {
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unexpected tokens after DDL statement".into()));
    }
    Ok(())
}

pub(super) fn optional_if_exists(parser: &mut Parser) -> Result<bool, ParseError> {
    if !parser.keyword("IF") {
        return Ok(false);
    }
    if !parser.keyword("EXISTS") {
        return Err(ParseError("expected IF EXISTS".into()));
    }
    Ok(true)
}

pub(super) fn optional_if_not_exists(parser: &mut Parser) -> Result<bool, ParseError> {
    if !parser.keyword("IF") {
        return Ok(false);
    }
    if !parser.keyword("NOT") || !parser.keyword("EXISTS") {
        return Err(ParseError("expected IF NOT EXISTS".into()));
    }
    Ok(true)
}

pub(super) fn parse_column_definitions(parser: &mut Parser) -> Result<Vec<ColumnDef>, ParseError> {
    let mut columns = Vec::new();
    let mut names = HashSet::new();
    loop {
        if columns.len() == 128 {
            return Err(ParseError("DDL supports at most 128 columns".into()));
        }
        let column_name = identifier(parser)?;
        if !names.insert(column_name.clone()) {
            return Err(ParseError(format!("duplicate DDL column: {column_name}")));
        }
        let column_type = data_type(parser, 0)?;
        columns.push(ColumnDef {
            name: column_name,
            data_type: column_type,
        });
        if !parser.take(Token::Comma) {
            break;
        }
    }
    Ok(columns)
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    #[test]
    fn float_alias_uses_supported_storage_type() {
        let parsed = crate::ddl::parse_ddl("CREATE TABLE values_table (score FLOAT)")
            .unwrap()
            .unwrap();
        let crate::ddl::Ddl::CreateTable { columns, .. } = parsed else {
            panic!("expected a CREATE TABLE");
        };
        assert_eq!(columns[0].data_type, "Float32");
    }
}
#[cfg(test)]
mod varchar_alias_tests {
    use super::*;

    #[test]
    fn text_aliases_use_string_storage() {
        let parsed = crate::ddl::parse_ddl(
            "CREATE TABLE labels (short_name VARCHAR, initial CHARACTER)",
        )
        .unwrap()
        .unwrap();
        let crate::ddl::Ddl::CreateTable { columns, .. } = parsed else {
            panic!("expected a CREATE TABLE");
        };
        assert_eq!(columns[0].data_type, "String");
        assert_eq!(columns[1].data_type, "String");
    }
}
