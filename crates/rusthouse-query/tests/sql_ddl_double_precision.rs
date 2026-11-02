use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn double_precision_is_the_same_type_as_double() {
    let Some(Ddl::CreateTable { columns, .. }) =
        parse_ddl("CREATE TABLE metrics (a DOUBLE PRECISION, b DOUBLE)").unwrap()
    else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(columns[0].data_type, "Float64");
    assert_eq!(columns[1].data_type, "Float64");
}

#[test]
fn double_precision_works_with_alter_column() {
    let Some(Ddl::AddColumn { column, .. }) =
        parse_ddl("ALTER TABLE metrics ADD COLUMN value DOUBLE PRECISION").unwrap()
    else {
        panic!("expected ALTER TABLE ADD COLUMN");
    };
    assert_eq!(column.data_type, "Float64");
}
