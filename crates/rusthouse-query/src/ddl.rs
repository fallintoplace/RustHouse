use super::{ParseError, Parser, Token, tokenize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ddl {
    ShowTables,
    ShowCreateTable {
        name: String,
    },
    DescribeTable {
        name: String,
    },
    CreateTable {
        name: String,
        columns: Vec<ColumnDef>,
        if_not_exists: bool,
    },
    DropTable {
        name: String,
        if_exists: bool,
    },
    TruncateTable {
        name: String,
    },
    OptimizeTable {
        name: String,
    },
    AddColumn {
        table: String,
        column: ColumnDef,
        if_not_exists: bool,
    },
    DropColumn {
        table: String,
        column: String,
        if_exists: bool,
    },
    RenameColumn {
        table: String,
        from: String,
        to: String,
    },
    ModifyColumn {
        table: String,
        column: ColumnDef,
    },
    RenameTable {
        from: String,
        to: String,
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

fn finish_ddl(parser: &mut Parser) -> Result<(), ParseError> {
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unexpected tokens after DDL statement".into()));
    }
    Ok(())
}

fn optional_if_exists(parser: &mut Parser) -> Result<bool, ParseError> {
    if !parser.keyword("IF") {
        return Ok(false);
    }
    if !parser.keyword("EXISTS") {
        return Err(ParseError("expected IF EXISTS".into()));
    }
    Ok(true)
}

fn optional_if_not_exists(parser: &mut Parser) -> Result<bool, ParseError> {
    if !parser.keyword("IF") {
        return Ok(false);
    }
    if !parser.keyword("NOT") || !parser.keyword("EXISTS") {
        return Err(ParseError("expected IF NOT EXISTS".into()));
    }
    Ok(true)
}

/// Recognize only supported schema-only DDL. SELECT is parsed by the existing parser.
pub fn parse_ddl(sql: &str) -> Result<Option<Ddl>, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    if parser.keyword("DESCRIBE") {
        if !parser.keyword("TABLE") {
            return Err(ParseError("expected TABLE after DESCRIBE".into()));
        }
        let name = identifier(&mut parser)?;
        finish_ddl(&mut parser)?;
        return Ok(Some(Ddl::DescribeTable { name }));
    }
    if parser.keyword("SHOW") {
        let statement = if parser.keyword("TABLES") {
            Ddl::ShowTables
        } else if parser.keyword("CREATE") {
            if !parser.keyword("TABLE") {
                return Err(ParseError("expected TABLE after SHOW CREATE".into()));
            }
            Ddl::ShowCreateTable {
                name: identifier(&mut parser)?,
            }
        } else {
            return Err(ParseError(
                "expected TABLES or CREATE TABLE after SHOW".into(),
            ));
        };
        finish_ddl(&mut parser)?;
        return Ok(Some(statement));
    }
    if parser.keyword("RENAME") {
        if !parser.keyword("TABLE") {
            return Err(ParseError("expected TABLE after RENAME".into()));
        }
        let from = identifier(&mut parser)?;
        if !parser.keyword("TO") {
            return Err(ParseError("expected TO after source table".into()));
        }
        let to = identifier(&mut parser)?;
        finish_ddl(&mut parser)?;
        return Ok(Some(Ddl::RenameTable { from, to }));
    }
    if parser.keyword("ALTER") {
        if !parser.keyword("TABLE") {
            return Err(ParseError("expected TABLE after ALTER".into()));
        }
        let table = identifier(&mut parser)?;
        if parser.keyword("RENAME") {
            if !parser.keyword("COLUMN") {
                return Err(ParseError("expected COLUMN after RENAME".into()));
            }
            let from = identifier(&mut parser)?;
            if !parser.keyword("TO") {
                return Err(ParseError("expected TO after source column".into()));
            }
            let to = identifier(&mut parser)?;
            finish_ddl(&mut parser)?;
            return Ok(Some(Ddl::RenameColumn { table, from, to }));
        }
        if parser.keyword("MODIFY") {
            if !parser.keyword("COLUMN") {
                return Err(ParseError("expected COLUMN after MODIFY".into()));
            }
            let name = identifier(&mut parser)?;
            let data_type = data_type(&mut parser, 0)?;
            finish_ddl(&mut parser)?;
            return Ok(Some(Ddl::ModifyColumn {
                table,
                column: ColumnDef { name, data_type },
            }));
        }
        if parser.keyword("DROP") {
            if !parser.keyword("COLUMN") {
                return Err(ParseError("expected COLUMN after DROP".into()));
            }
            let if_exists = optional_if_exists(&mut parser)?;
            let column = identifier(&mut parser)?;
            finish_ddl(&mut parser)?;
            return Ok(Some(Ddl::DropColumn {
                table,
                column,
                if_exists,
            }));
        }
        if !parser.keyword("ADD") || !parser.keyword("COLUMN") {
            return Err(ParseError(
                "expected ADD COLUMN or DROP COLUMN after table name".into(),
            ));
        }
        let if_not_exists = optional_if_not_exists(&mut parser)?;
        let name = identifier(&mut parser)?;
        let data_type = data_type(&mut parser, 0)?;
        finish_ddl(&mut parser)?;
        return Ok(Some(Ddl::AddColumn {
            table,
            column: ColumnDef { name, data_type },
            if_not_exists,
        }));
    }
    if parser.keyword("OPTIMIZE") {
        if !parser.keyword("TABLE") {
            return Err(ParseError("expected TABLE after OPTIMIZE".into()));
        }
        let name = identifier(&mut parser)?;
        finish_ddl(&mut parser)?;
        return Ok(Some(Ddl::OptimizeTable { name }));
    }
    if parser.keyword("TRUNCATE") {
        if !parser.keyword("TABLE") {
            return Err(ParseError("expected TABLE after TRUNCATE".into()));
        }
        let name = identifier(&mut parser)?;
        finish_ddl(&mut parser)?;
        return Ok(Some(Ddl::TruncateTable { name }));
    }
    if parser.keyword("DROP") {
        if !parser.keyword("TABLE") {
            return Err(ParseError("expected TABLE after DROP".into()));
        }
        let if_exists = optional_if_exists(&mut parser)?;
        let name = identifier(&mut parser)?;
        finish_ddl(&mut parser)?;
        return Ok(Some(Ddl::DropTable { name, if_exists }));
    }
    if !parser.keyword("CREATE") {
        return Ok(None);
    }
    if !parser.keyword("TABLE") {
        return Err(ParseError("expected TABLE after CREATE".into()));
    }
    let if_not_exists = optional_if_not_exists(&mut parser)?;
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
    finish_ddl(&mut parser)?;
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
    fn parses_show_create_table_and_rejects_bad_forms() {
        assert_eq!(
            parse_ddl("SHOW CREATE TABLE events;").unwrap(),
            Some(Ddl::ShowCreateTable {
                name: "events".into()
            })
        );
        assert_eq!(parse_ddl("show tables").unwrap(), Some(Ddl::ShowTables));
        for invalid in [
            "SHOW CREATE events",
            "SHOW CREATE TABLE",
            "SHOW CREATE TABLE events; DROP TABLE events",
            "SHOW CREATE TABLE events extra",
            "SHOW CREATE TABLE events CREATE",
        ] {
            assert!(parse_ddl(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn parses_create_with_nested_types_and_idempotency() {
        assert_eq!(
            parse_ddl(
                "create table if not exists events (id uint64, payload Nullable(Array(String)));"
            )
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
    fn parses_rename_table_and_rejects_trailing_sql() {
        assert_eq!(
            parse_ddl("RENAME TABLE events TO archived_events;").unwrap(),
            Some(Ddl::RenameTable {
                from: "events".into(),
                to: "archived_events".into(),
            })
        );
        for sql in [
            "RENAME events TO archived_events",
            "RENAME TABLE events archived_events",
            "RENAME TABLE events TO archived_events, other TO t",
            "RENAME TABLE events TO archived_events; DROP TABLE archived_events",
        ] {
            assert!(parse_ddl(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_drop_column_and_rejects_bad_forms() {
        assert_eq!(
            parse_ddl("ALTER TABLE events DROP COLUMN IF EXISTS age;").unwrap(),
            Some(Ddl::DropColumn {
                table: "events".into(),
                column: "age".into(),
                if_exists: true,
            })
        );
        for sql in [
            "ALTER TABLE events DROP age",
            "ALTER TABLE events DROP COLUMN",
            "ALTER TABLE events DROP COLUMN IF NOT EXISTS age",
            "ALTER TABLE events DROP COLUMN age, info",
        ] {
            assert!(parse_ddl(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_add_column_and_rejects_unsupported_alter_syntax() {
        assert_eq!(
            parse_ddl("ALTER TABLE events ADD COLUMN IF NOT EXISTS info Nullable(String);")
                .unwrap(),
            Some(Ddl::AddColumn {
                table: "events".into(),
                column: ColumnDef {
                    name: "info".into(),
                    data_type: "Nullable(String)".into(),
                },
                if_not_exists: true,
            })
        );
        for sql in [
            "ALTER TABLE events ADD info String",
            "ALTER TABLE events ADD COLUMN id",
            "ALTER TABLE events ADD COLUMN id Int64 AFTER info",
            "ALTER TABLE events ADD COLUMN IF EXISTS info String",
        ] {
            assert!(parse_ddl(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_truncate_and_rejects_malformed_or_multiple_statements() {
        assert_eq!(
            parse_ddl("TRUNCATE TABLE events;").unwrap(),
            Some(Ddl::TruncateTable {
                name: "events".into(),
            })
        );
        for statement in [
            "TRUNCATE events",
            "TRUNCATE TABLE",
            "TRUNCATE TABLE events RESTART IDENTITY",
            "TRUNCATE TABLE events; DROP TABLE events",
        ] {
            assert!(parse_ddl(statement).is_err(), "{statement}");
        }
    }

    #[test]
    fn parses_drop_and_rejects_extra_statements() {
        assert_eq!(
            parse_ddl("drop table if exists events;").unwrap(),
            Some(Ddl::DropTable {
                name: "events".into(),
                if_exists: true,
            })
        );
        assert!(matches!(
            parse_ddl("DROP TABLE events").unwrap(),
            Some(Ddl::DropTable {
                if_exists: false,
                ..
            })
        ));
        for sql in [
            "DROP TABLE",
            "DROP events",
            "DROP TABLE IF NOT EXISTS events",
            "DROP TABLE events; DROP TABLE other",
        ] {
            assert!(parse_ddl(sql).is_err(), "{sql}");
        }
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

    #[test]
    fn parses_modify_column_and_rejects_unsupported_forms() {
        assert_eq!(
            parse_ddl("ALTER TABLE events MODIFY COLUMN payload Nullable(String);").unwrap(),
            Some(Ddl::ModifyColumn {
                table: "events".into(),
                column: ColumnDef {
                    name: "payload".into(),
                    data_type: "Nullable(String)".into(),
                },
            })
        );
        for sql in [
            "ALTER TABLE events MODIFY payload String",
            "ALTER TABLE events MODIFY COLUMN payload",
            "ALTER TABLE events MODIFY COLUMN payload Unknown",
            "ALTER TABLE events MODIFY COLUMN payload String, id UInt64",
            "ALTER TABLE events MODIFY COLUMN payload String; DROP TABLE events",
        ] {
            assert!(parse_ddl(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_column_renaming_and_rejects_unsupported_forms() {
        assert_eq!(
            parse_ddl("ALTER TABLE events RENAME COLUMN old_name TO new_name;").unwrap(),
            Some(Ddl::RenameColumn {
                table: "events".into(),
                from: "old_name".into(),
                to: "new_name".into(),
            })
        );
        for sql in [
            "ALTER TABLE events RENAME old_name TO new_name",
            "ALTER TABLE events RENAME COLUMN old_name new_name",
            "ALTER TABLE events RENAME COLUMN old_name TO",
            "ALTER TABLE events RENAME COLUMN old_name TO new_name, x TO y",
        ] {
            assert!(parse_ddl(sql).is_err(), "{sql}");
        }
    }
}
