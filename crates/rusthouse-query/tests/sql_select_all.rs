use rusthouse_query::{execute, parse_join, parse_select};

#[test]
fn explicit_all_keeps_regular_rows() {
    let query = parse_select(
        "SELECT ALL number FROM numbers(4) ORDER BY number DESC LIMIT 2",
    )
    .unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "number\n3\n2\n");
}

#[test]
fn explicit_all_is_allowed_before_a_join_projection() {
    assert!(parse_join(
        "SELECT ALL u.id FROM users u INNER JOIN teams t ON u.team_id = t.id"
    )
    .unwrap()
    .is_some());
}
