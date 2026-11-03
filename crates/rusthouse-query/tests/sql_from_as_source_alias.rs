use rusthouse_query::{execute, parse_select};

#[test]
fn unused_source_alias_does_not_change_generated_rows() {
    let sql = "SELECT number FROM numbers(4) AS series WHERE number >= 2 ORDER BY number";
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "number\n2\n3\n");
}

#[test]
fn named_table_alias_preserves_unqualified_column_bindings() {
    assert!(parse_select("SELECT id FROM events AS e WHERE id > 1").is_ok());
    assert!(parse_select("SELECT count(*) FROM events AS source").is_ok());
    assert!(parse_select("SELECT number FROM numbers(3) AS").is_err());
    assert!(parse_select("SELECT number FROM numbers(3) AS 7").is_err());
}
