use rusthouse_query::{execute, parse_select};

#[test]
fn upper_case_order_by_alias_resolves_to_projected_column() {
    let query = parse_select("SELECT number AS value FROM numbers(4) ORDER BY VALUE DESC").unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "value\n3\n2\n1\n0\n");
}

#[test]
fn alias_case_does_not_mask_a_nonexistent_sort_column() {
    let query = parse_select("SELECT number AS value FROM numbers(4) ORDER BY missing").unwrap();
    let mut output = Vec::new();
    assert!(execute(&query, None, &mut output).is_err());
}
