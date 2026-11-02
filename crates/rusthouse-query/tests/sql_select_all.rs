use rusthouse_query::{execute, parse_select};

#[test]
fn explicit_all_matches_default_projection() {
    let plain = parse_select("SELECT number FROM numbers(4) ORDER BY number").unwrap();
    let explicit = parse_select("SELECT ALL number FROM numbers(4) ORDER BY number").unwrap();
    assert_eq!(plain, explicit);

    let mut output = Vec::new();
    execute(&explicit, None, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "number\n0\n1\n2\n3\n");
}

#[test]
fn explicit_all_cannot_combine_with_distinct() {
    assert!(parse_select("SELECT ALL DISTINCT number FROM numbers(4)").is_err());
}
