use rusthouse_query::parse_ddl;

#[test]
fn jsonb_declarations_use_existing_json_storage() {
    let json = parse_ddl("CREATE TABLE docs (payload JSON)").unwrap();
    let jsonb = parse_ddl("CREATE TABLE docs (payload JSONB)").unwrap();
    assert_eq!(jsonb, json);
    assert_eq!(
        parse_ddl("ALTER TABLE docs ADD COLUMN extra JSONB").unwrap(),
        parse_ddl("ALTER TABLE docs ADD COLUMN extra JSON").unwrap()
    );
}
