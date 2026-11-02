use rusthouse_query::{Source, parse_select};
#[test]
fn dual_is_one_row_for_constant_select() {
    let query = parse_select("SELECT 7 AS lucky FROM DUAL").unwrap();
    assert!(query.source.is_none());
    let table = parse_select("SELECT value FROM dual").unwrap();
    assert!(matches!(table.source, Some(Source::Table(ref name)) if name == "dual"));
}
