use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn create_table_as_existing_uses_schema_clone_path() {
    for sql in [
        "CREATE TABLE archived AS events",
        "CREATE TABLE IF NOT EXISTS archived AS events;",
        "create table archived as events",
    ] {
        let ddl = parse_ddl(sql).unwrap().unwrap();
        assert_eq!(
            ddl,
            Ddl::CreateTableLike {
                name: "archived".into(),
                source: "events".into(),
                if_not_exists: sql.to_ascii_uppercase().contains("IF NOT EXISTS"),
            },
            "{sql}"
        );
    }
}

#[test]
fn create_as_rejects_missing_or_extra_statements() {
    for sql in [
        "CREATE TABLE archived AS",
        "CREATE TABLE archived AS events (id UInt64)",
        "CREATE TABLE archived AS events; DROP TABLE events",
    ] {
        assert!(parse_ddl(sql).is_err(), "{sql}");
    }
}
