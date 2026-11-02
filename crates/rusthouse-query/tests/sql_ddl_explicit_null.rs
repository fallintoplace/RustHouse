use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn create_table_null_modifier_uses_nullable_storage() {
    let Some(Ddl::CreateTable { columns, .. }) =
        parse_ddl("CREATE TABLE sample (id INT NULL, name TEXT NULL, flag BOOL)").unwrap()
    else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(columns[0].data_type, "Nullable(Int32)");
    assert_eq!(columns[1].data_type, "Nullable(String)");
    assert_eq!(columns[2].data_type, "Bool");
}

#[test]
fn explicitly_nullable_columns_do_not_gain_an_extra_layer() {
    let Some(Ddl::CreateTable { columns, .. }) =
        parse_ddl("CREATE TABLE t (x Nullable(Int32) NULL)").unwrap()
    else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(columns[0].data_type, "Nullable(Int32)");
}
