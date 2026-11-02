use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn character_varying_uses_the_string_storage_type() {
    let Some(Ddl::CreateTable { columns, .. }) =
        parse_ddl("CREATE TABLE labels (title CHARACTER VARYING, alias CHARACTER)").unwrap()
    else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(columns[0].data_type, "String");
    assert_eq!(columns[1].data_type, "String");
}

#[test]
fn character_varying_is_allowed_in_alter() {
    let Some(Ddl::AddColumn { column, .. }) =
        parse_ddl("ALTER TABLE labels ADD COLUMN description CHARACTER VARYING").unwrap()
    else {
        panic!("expected ALTER ADD COLUMN");
    };
    assert_eq!(column.data_type, "String");
}
