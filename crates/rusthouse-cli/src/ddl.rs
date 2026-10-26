use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Column {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Statement {
    CreateTable {
        name: String,
        if_not_exists: bool,
        columns: Vec<Column>,
    },
}

struct Parser<'a> {
    tokens: Vec<&'a str>,
    index: usize,
}

impl<'a> Parser<'a> {
    fn take(&mut self, value: &str) -> bool {
        if self.tokens.get(self.index).is_some_and(|token| {
            token.eq_ignore_ascii_case(value)
        }) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, value: &str) -> Result<(), String> {
        if self.take(value) {
            Ok(())
        } else {
            Err(format!("expected {value}"))
        }
    }

    fn identifier(&mut self) -> Result<String, String> {
        let value = self.tokens.get(self.index).ok_or("expected an identifier")?;
        if !valid_identifier(value) {
            return Err("expected an identifier".into());
        }
        self.index += 1;
        Ok((*value).into())
    }

    fn finish(&mut self) -> Result<(), String> {
        self.take(";");
        if self.index != self.tokens.len() {
            return Err("unexpected tokens after DDL statement".into());
        }
        Ok(())
    }
}

fn valid_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn tokenize(sql: &str) -> Result<Vec<&str>, String> {
    if sql.len() > 4096 {
        return Err("DDL statement exceeds the 4096-byte limit".into());
    }
    let bytes = sql.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        if bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
        } else if matches!(bytes[index], b'(' | b')' | b',' | b';') {
            index += 1;
        } else {
            return Err(format!("unsupported DDL character at byte {index}"));
        }
        tokens.push(&sql[start..index]);
    }
    Ok(tokens)
}

fn canonical_type(value: &str) -> Option<&'static str> {
    match value.to_ascii_uppercase().as_str() {
        "UINT8" => Some("UInt8"),
        "UINT16" => Some("UInt16"),
        "UINT32" => Some("UInt32"),
        "UINT64" => Some("UInt64"),
        "INT8" => Some("Int8"),
        "INT16" => Some("Int16"),
        "INT32" => Some("Int32"),
        "INT64" => Some("Int64"),
        "FLOAT32" => Some("Float32"),
        "FLOAT64" => Some("Float64"),
        "STRING" => Some("String"),
        "DATE" => Some("Date"),
        "DATETIME" => Some("DateTime"),
        _ => None,
    }
}

pub(super) fn parse_ddl(sql: &str) -> Result<Option<Statement>, String> {
    let first = sql.trim_start().split(|c: char| !c.is_ascii_alphabetic()).next();
    if !first.is_some_and(|value| value.eq_ignore_ascii_case("CREATE")) {
        return Ok(None);
    }
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    parser.expect("CREATE")?;
    parser.expect("TABLE")?;
    let if_not_exists = if parser.take("IF") {
        parser.expect("NOT")?;
        parser.expect("EXISTS")?;
        true
    } else {
        false
    };
    let name = parser.identifier()?;
    parser.expect("(")?;
    let mut columns: Vec<Column> = Vec::new();
    loop {
        let column_name = parser.identifier()?;
        if columns.iter().any(|col| col.name.eq_ignore_ascii_case(&column_name)) {
            return Err(format!("duplicate column: {column_name}"));
        }
        let raw_type = parser.identifier()?;
        let data_type = canonical_type(&raw_type)
            .ok_or_else(|| format!("unsupported column type: {raw_type}"))?;
        columns.push(Column {
            name: column_name,
            data_type: data_type.into(),
        });
        if parser.take(")") {
            break;
        }
        parser.expect(",")?;
    }
    parser.finish()?;
    Ok(Some(Statement::CreateTable {
        name,
        if_not_exists,
        columns,
    }))
}

pub(super) fn execute_ddl(statement: Statement, root: &Path) -> Result<(), String> {
    match statement {
        Statement::CreateTable {
            name,
            if_not_exists,
            columns,
        } => {
            fs::create_dir_all(root)
                .map_err(|error| format!("cannot create catalog {}: {error}", root.display()))?;
            let path = root.join(format!("{name}.rh"));
            let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => file,
                Err(error) if if_not_exists && error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Ok(());
                }
                Err(error) => {
                    return Err(format!("cannot create table {name}: {error}"));
                }
            };
            let mut contents = String::from("RustHouseTableV1\n");
            for column in &columns {
                contents.push_str(&format!("{} {}\n", column.name, column.data_type));
            }
            if let Err(error) = file.write_all(contents.as_bytes()).and_then(|_| file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(&path);
                return Err(format!("cannot write table {name}: {error}"));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_create_table_with_canonical_types() {
        let statement = parse_ddl("create table if not exists events (id uint64, d datetime);")
            .unwrap()
            .unwrap();
        assert_eq!(
            statement,
            Statement::CreateTable {
                name: "events".into(),
                if_not_exists: true,
                columns: vec![
                    Column {
                        name: "id".into(),
                        data_type: "UInt64".into(),
                    },
                    Column {
                        name: "d".into(),
                        data_type: "DateTime".into(),
                    },
                ],
            }
        );
    }

    #[test]
    fn rejects_unsafe_names_duplicate_columns_and_unsupported_syntax() {
        for sql in [
            "CREATE TABLE t ()",
            "CREATE TABLE t (id UInt64, ID Int64)",
            "CREATE TABLE ../bad (id UInt64)",
            "CREATE TABLE t (x Nullable)",
            "CREATE TABLE t (x UInt64) ENGINE = MergeTree",
            "CREATE TABLE t (x UInt64); DROP TABLE t",
        ] {
            assert!(parse_ddl(sql).is_err(), "{sql}");
        }
        assert!(parse_ddl("SELECT 1").unwrap().is_none());
    }
}
