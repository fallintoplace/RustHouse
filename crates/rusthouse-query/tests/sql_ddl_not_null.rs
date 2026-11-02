use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn explicit_not_null_keeps_nonnullable_schema() {
    let Some(Ddl::CreateTable { columns, .. }) =
        parse_ddl("CREATE TABLE users (id INTEGER NOT NULL, name TEXT NOT NULL)").unwrap()
    else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(columns[0].data_type, "Int32");
    assert_eq!(columns[1].data_type, "String");
}

#[test]
fn rejects_invalid_nullability_constraints() {
    assert!(parse_ddl("CREATE TABLE t (id Nullable(Int32) NOT NULL)").is_err());
    assert!(parse_ddl("CREATE TABLE t (id INTEGER NOT)").is_err());
}
