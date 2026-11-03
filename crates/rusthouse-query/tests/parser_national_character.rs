use rusthouse_query::parse_ddl;

#[test]
fn national_character_types_use_string_storage() {
    let national = parse_ddl(
        "CREATE TABLE t (a NATIONAL CHARACTER, b NATIONAL CHARACTER VARYING, c NATIONAL VARCHAR)"
    )
    .unwrap();
    let equivalent = parse_ddl("CREATE TABLE t (a NCHAR, b NVARCHAR, c NVARCHAR)").unwrap();
    assert_eq!(national, equivalent);
    for invalid in [
        "CREATE TABLE t (a NATIONAL)",
        "CREATE TABLE t (a NATIONAL INTEGER)",
    ] {
        assert!(parse_ddl(invalid).is_err(), "{invalid}");
    }
}
