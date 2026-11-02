use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn common_character_aliases_use_string_storage() {
    let Some(Ddl::CreateTable { columns, .. }) =
        parse_ddl("CREATE TABLE strings (c CHAR, n NCHAR, v NVARCHAR, t TEXT)").unwrap()
    else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(
        columns.iter().map(|column| column.data_type.as_str()).collect::<Vec<_>>(),
        ["String", "String", "String", "String"]
    );
}

#[test]
fn alter_column_supports_nvarchar() {
    let Some(Ddl::AddColumn { column, .. }) =
        parse_ddl("ALTER TABLE strings ADD COLUMN title NVARCHAR").unwrap()
    else {
        panic!("expected ALTER TABLE");
    };
    assert_eq!(column.data_type, "String");
}
