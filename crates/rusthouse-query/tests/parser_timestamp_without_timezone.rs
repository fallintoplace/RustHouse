use rusthouse_query::parse_ddl;

#[test]
fn timestamp_without_time_zone_uses_datetime() {
    let standard = parse_ddl("CREATE TABLE t (created TIMESTAMP WITHOUT TIME ZONE)").unwrap();
    let existing = parse_ddl("CREATE TABLE t (created TIMESTAMP)").unwrap();
    assert_eq!(standard, existing);
    for invalid in [
        "CREATE TABLE t (created TIMESTAMP WITHOUT)",
        "CREATE TABLE t (created TIMESTAMP WITHOUT TIME)",
        "CREATE TABLE t (created TIMESTAMP WITH TIME ZONE)",
    ] {
        assert!(parse_ddl(invalid).is_err(), "{invalid}");
    }
}
