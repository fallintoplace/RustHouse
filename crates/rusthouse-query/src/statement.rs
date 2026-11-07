//! Classify local database statements without duplicating parser precedence.

use crate::{
    CopyFrom, Ddl, Insert, InsertSelect, ParseError, Select, Update, parse_copy_from,
    parse_ddl, parse_delete, parse_insert, parse_insert_select, parse_update,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseStatement {
    Ddl(Ddl),
    Copy(CopyFrom),
    InsertSelect(InsertSelect),
    Insert(Insert),
    Delete(Select),
    Update(Update),
}

pub fn parse_database_statement(sql: &str) -> Result<Option<DatabaseStatement>, ParseError> {
    if let Some(value) = parse_ddl(sql)? {
        return Ok(Some(DatabaseStatement::Ddl(value)));
    }
    if let Some(value) = parse_copy_from(sql)? {
        return Ok(Some(DatabaseStatement::Copy(value)));
    }
    if let Some(value) = parse_insert_select(sql)? {
        return Ok(Some(DatabaseStatement::InsertSelect(value)));
    }
    if let Some(value) = parse_insert(sql)? {
        return Ok(Some(DatabaseStatement::Insert(value)));
    }
    if let Some(value) = parse_delete(sql)? {
        return Ok(Some(DatabaseStatement::Delete(value)));
    }
    if let Some(value) = parse_update(sql)? {
        return Ok(Some(DatabaseStatement::Update(value)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_local_database_ddl_before_select_fallback() {
        assert!(matches!(parse_database_statement("SHOW TABLES").unwrap(), Some(DatabaseStatement::Ddl(Ddl::ShowTables))));
        assert!(parse_database_statement("SELECT 1").unwrap().is_none());
    }
}
