use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn double_precision_uses_float64() {
    let Some(Ddl::CreateTable { columns, .. }) =
        parse_ddl("CREATE TABLE metrics (value DOUBLE PRECISION, fallback DOUBLE)").unwrap()
    else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(columns[0].data_type, "Float64");
    assert_eq!(columns[1].data_type, "Float64");
}

#[test]
fn double_precision_is_allowed_in_alter_and_rejects_extra_words() {
    let Some(Ddl::AddColumn { column, .. }) =
        parse_ddl("ALTER TABLE metrics ADD COLUMN score DOUBLE PRECISION").unwrap()
    else {
        panic!("expected ADD COLUMN");
    };
    assert_eq!(column.data_type, "Float64");
    assert!(parse_ddl("CREATE TABLE bad (score DOUBLE PRECISION EXTRA)").is_err());
}
