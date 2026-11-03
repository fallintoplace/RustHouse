use rusthouse_query::parse_ddl;

#[test]
fn char_varying_matches_character_varying() {
    let standard = parse_ddl("CREATE TABLE t (label CHARACTER VARYING)").unwrap();
    let short = parse_ddl("CREATE TABLE t (label CHAR VARYING)").unwrap();
    assert_eq!(short, standard);
    assert_eq!(
        parse_ddl("CREATE TABLE t (label CHAR)").unwrap(),
        parse_ddl("CREATE TABLE t (label VARCHAR)").unwrap()
    );
}
