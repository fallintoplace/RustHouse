use rusthouse_query::{execute, parse_select};

#[test]
fn unquoted_group_by_matches_selected_key_case_insensitively() {
    let query = parse_select(
        "SELECT number, COUNT(*) FROM numbers(3) GROUP BY NUMBER ORDER BY number"
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "number\tcount\n0\t1\n1\t1\n2\t1\n");
    assert!(parse_select("SELECT number, COUNT(*) FROM numbers(3) GROUP BY other").is_err());
}

#[test]
fn multikey_group_by_uses_same_identifier_folding() {
    assert!(parse_select("SELECT city, type, COUNT(*) FROM logs GROUP BY CITY, TYPE").is_ok());
    assert!(parse_select("SELECT city, type, COUNT(*) FROM logs GROUP BY TYPE, CITY").is_err());
}
