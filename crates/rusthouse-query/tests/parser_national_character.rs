use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn national_character_types_use_string_storage() {
    let Some(Ddl::CreateTable { columns, .. }) =
        parse_ddl("CREATE TABLE names (first NATIONAL CHARACTER, last NATIONAL CHAR VARYING)").unwrap()
    else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(columns[0].data_type, "String");
    assert_eq!(columns[1].data_type, "String");
}

#[test]
fn national_character_is_allowed_in_alter_but_requires_character() {
    let Some(Ddl::AddColumn { column, .. }) =
        parse_ddl("ALTER TABLE names ADD COLUMN title NATIONAL CHARACTER VARYING").unwrap()
    else {
        panic!("expected ADD COLUMN");
    };
    assert_eq!(column.data_type, "String");
    assert!(parse_ddl("CREATE TABLE bad (name NATIONAL VARYING)").is_err());
}
