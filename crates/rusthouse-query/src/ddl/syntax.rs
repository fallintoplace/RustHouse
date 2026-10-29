use super::*;

const MAX_DDL_IDENTIFIER_BYTES: usize = 128;
const MAX_DDL_COLUMNS: usize = 128;

pub(super) fn identifier(parser: &mut Parser) -> Result<String, ParseError> {
    let value = parser.word()?;
    if value.len() > MAX_DDL_IDENTIFIER_BYTES {
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
        "UUID" => "UUID",
        "DATE" => "Date",
        "DATETIME" => "DateTime",
        "BOOL" => "Bool",
        "NULLABLE" => "Nullable",
        "ARRAY" => "Array",
        "MAP" => "Map",
        "JSON" => "JSON",
        _ => return None,
    })
}

pub(super) fn data_type(parser: &mut Parser, nesting: usize) -> Result<String, ParseError> {
    let name = parser.word()?;
    let canonical = canonical_type(&name)
        .ok_or_else(|| ParseError(format!("unsupported DDL column type: {name}")))?;
    if matches!(canonical, "Nullable" | "Array" | "Map") {
        if nesting >= 4 {
            return Err(ParseError("DDL type nesting exceeds 4 levels".into()));
        }
        if !parser.take(Token::LParen) {
            return Err(ParseError(format!("{canonical} requires type arguments")));
        }
        let first = data_type(parser, nesting + 1)?;
        let result = if canonical == "Map" {
            if !matches!(
                first.as_str(),
                "String"
                    | "Int8"
                    | "Int16"
                    | "Int32"
                    | "Int64"
                    | "Int128"
                    | "UInt8"
                    | "UInt16"
                    | "UInt32"
                    | "UInt64"
                    | "UInt128"
            ) {
                return Err(ParseError(
                    "Map keys must be non-nullable String or integer types".into(),
                ));
            }
            if !parser.take(Token::Comma) {
                return Err(ParseError("Map requires a key and value type".into()));
            }
            let value = data_type(parser, nesting + 1)?;
            format!("Map({first}, {value})")
        } else {
            format!("{canonical}({first})")
        };
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after column type".into()));
        }
        Ok(result)
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
        if columns.len() == MAX_DDL_COLUMNS {
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

    #[test]
    fn text_aliases_use_string_storage() {
        let parsed =
            crate::ddl::parse_ddl("CREATE TABLE labels (short_name VARCHAR, initial CHARACTER)")
                .unwrap()
                .unwrap();
        let crate::ddl::Ddl::CreateTable { columns, .. } = parsed else {
            panic!("expected a CREATE TABLE");
        };
        assert_eq!(columns[0].data_type, "String");
        assert_eq!(columns[1].data_type, "String");
    }
}

#[cfg(test)]
mod complex_type_tests {
    use crate::ddl::{Ddl, parse_ddl};

    fn types(sql: &str) -> Vec<String> {
        let Some(Ddl::CreateTable { columns, .. }) = parse_ddl(sql).unwrap() else {
            panic!("expected CREATE TABLE");
        };
        columns.into_iter().map(|column| column.data_type).collect()
    }

    #[test]
    fn parses_map_array_and_json_type_declarations() {
        assert_eq!(
            types("CREATE TABLE data (attrs map(varchar, uint64), tags array(string), doc json)"),
            ["Map(String, UInt64)", "Array(String)", "JSON"]
        );
        assert_eq!(
            types("CREATE TABLE t (nested Array(Map(String, Array(Nullable(Int32)))))"),
            ["Array(Map(String, Array(Nullable(Int32))))"]
        );
    }

    #[test]
    fn parses_complex_types_in_alter_column() {
        assert!(matches!(
            parse_ddl("ALTER TABLE data ADD COLUMN meta Map(UInt64, JSON)").unwrap(),
            Some(Ddl::AddColumn { column, .. }) if column.data_type == "Map(UInt64, JSON)"
        ));
        assert!(matches!(
            parse_ddl("ALTER TABLE data MODIFY COLUMN meta JSON").unwrap(),
            Some(Ddl::ModifyColumn { column, .. }) if column.data_type == "JSON"
        ));
    }

    #[test]
    fn rejects_incomplete_or_unsafe_map_types() {
        for sql in [
            "CREATE TABLE t (m Map)",
            "CREATE TABLE t (m Map())",
            "CREATE TABLE t (m Map(String))",
            "CREATE TABLE t (m Map(String,))",
            "CREATE TABLE t (m Map(String, UInt64, UInt8))",
            "CREATE TABLE t (m Map(Nullable(String), UInt8))",
            "CREATE TABLE t (m Map(Array(String), UInt8))",
            "CREATE TABLE t (m Map(Map(String, Int8), String))",
            "CREATE TABLE t (m Map(String, Array(Array(Array(Array(UInt8))))))",
            "CREATE TABLE t (j JSON(UInt8))",
        ] {
            assert!(parse_ddl(sql).is_err(), "{sql}");
        }
    }
}
