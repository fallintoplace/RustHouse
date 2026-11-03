use rusthouse_query::parse_ddl;

#[test]
fn common_text_type_aliases_share_string_storage() {
    let aliases =
        parse_ddl("CREATE TABLE t (a TINYTEXT, b MEDIUMTEXT, c LONGTEXT, d NTEXT, e LONGVARCHAR)")
            .unwrap();
    let baseline = parse_ddl("CREATE TABLE t (a TEXT, b TEXT, c TEXT, d TEXT, e TEXT)").unwrap();
    assert_eq!(aliases, baseline);
}
