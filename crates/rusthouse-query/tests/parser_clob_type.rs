use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn clob_maps_to_text_storage() {
    let ddl = parse_ddl("CREATE TABLE documents (body CLOB)").unwrap().unwrap();
    let Ddl::CreateTable { columns, .. } = ddl else { panic!("expected CREATE TABLE"); };
    assert_eq!(columns[0].data_type, "String");
}
