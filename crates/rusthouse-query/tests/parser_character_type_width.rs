use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn character_width_annotations_use_string_storage() {
    let sql = "CREATE TABLE labels (a VARCHAR(255), b CHAR(8), c NVARCHAR(64), d CHARACTER VARYING(128))";
    let Some(Ddl::CreateTable { columns, .. }) = parse_ddl(sql).unwrap() else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(
        columns.iter().map(|column| column.data_type.as_str()).collect::<Vec<_>>(),
        ["String", "String", "String", "String"]
    );
    let Some(Ddl::AddColumn { column, .. }) =
        parse_ddl("ALTER TABLE labels ADD COLUMN title VARCHAR(1024)").unwrap()
    else {
        panic!("expected ADD COLUMN");
    };
    assert_eq!(column.data_type, "String");
}

#[test]
fn character_width_must_be_a_positive_integer() {
    for decl in ["VARCHAR(0)", "CHAR(-1)", "NVARCHAR(1.5)", "VARCHAR(x)", "CHAR(2, 3)", "VARCHAR(4294967296)"] {
        let sql = format!("CREATE TABLE bad (name {decl})");
        assert!(parse_ddl(&sql).is_err(), "{sql}");
    }
    assert!(parse_ddl("CREATE TABLE bad (name STRING(4))").is_err());
}
