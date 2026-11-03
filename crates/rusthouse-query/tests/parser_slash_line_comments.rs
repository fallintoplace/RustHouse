use rusthouse_query::parse_select;

#[test]
fn clickhouse_slash_comments_end_at_the_next_line() {
    let with_comment = parse_select("SELECT 1 // note about the query\n;").unwrap();
    let plain = parse_select("SELECT 1;").unwrap();
    assert_eq!(with_comment.projection, plain.projection);
    assert_eq!(
        parse_select("SELECT 1//// another comment\n;").unwrap().projection,
        plain.projection
    );
    assert!(parse_select("SELECT 8 / 2").is_ok());
    assert!(parse_select("SELECT 1 /* existing block comment */").is_ok());
}
