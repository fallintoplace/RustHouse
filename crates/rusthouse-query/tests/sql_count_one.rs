use rusthouse_query::{execute, parse_select};

fn output(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut result = Vec::new();
    execute(&query, None, &mut result).unwrap();
    String::from_utf8(result).unwrap()
}

#[test]
fn count_one_matches_count_star_for_nonempty_and_empty_inputs() {
    for count in [0, 1, 7] {
        let one = format!("SELECT COUNT(1) FROM numbers({count})");
        let star = format!("SELECT COUNT(*) FROM numbers({count})");
        assert_eq!(output(&one), output(&star));
    }
}

#[test]
fn count_null_is_not_a_row_count() {
    assert!(parse_select("SELECT COUNT(NULL) FROM numbers(3)").is_err());
}
