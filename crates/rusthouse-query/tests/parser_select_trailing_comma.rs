use rusthouse_query::{Projection,parse_select};
#[test]
fn trailing_column_comma_is_allowed_before_from() {
    let query = parse_select("SELECT number, FROM numbers(3)").unwrap();
    assert_eq!(query.projection, Projection::Columns(vec!["number".into()]));
    let query = parse_select("SELECT id, name, FROM people").unwrap();
    assert_eq!(query.projection, Projection::Columns(vec!["id".into(), "name".into()]));
    assert!(parse_select("SELECT id,, FROM people").is_err());
}
