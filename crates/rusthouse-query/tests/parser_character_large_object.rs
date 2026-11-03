use rusthouse_query::parse_ddl;

#[test]
fn standard_character_large_object_maps_to_string() {
    assert_eq!(
        parse_ddl("CREATE TABLE articles (body CHARACTER LARGE OBJECT)").unwrap(),
        parse_ddl("CREATE TABLE articles (body CLOB)").unwrap()
    );
    assert_eq!(
        parse_ddl("ALTER TABLE articles ADD COLUMN summary CHARACTER LARGE OBJECT").unwrap(),
        parse_ddl("ALTER TABLE articles ADD COLUMN summary TEXT").unwrap()
    );
    assert!(parse_ddl("CREATE TABLE t (body CHARACTER LARGE)").is_err());
    assert!(parse_ddl("CREATE TABLE t (body CHARACTER LARGE WRONG)").is_err());
}
