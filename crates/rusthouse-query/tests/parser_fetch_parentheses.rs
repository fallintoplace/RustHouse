use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn fetch_accepts_parenthesized_row_count() {
    let query = parse_select(
        "SELECT number FROM numbers(5) ORDER BY number DESC FETCH FIRST (2) ROWS ONLY",
    ).unwrap();
    assert_eq!(query.limit, Some(2));
    assert_eq!(
        run("SELECT number FROM numbers(5) ORDER BY number DESC FETCH FIRST (2) ROWS ONLY"),
        "number\n4\n3\n"
    );
    assert_eq!(
        parse_select("SELECT number FROM numbers(5) FETCH NEXT (0) ROWS ONLY").unwrap().limit,
        Some(0)
    );
}

#[test]
fn fetch_parentheses_reject_bad_counts() {
    for sql in [
        "SELECT number FROM numbers(5) FETCH FIRST (2 ROWS ONLY",
        "SELECT number FROM numbers(5) FETCH FIRST (-1) ROWS ONLY",
        "SELECT number FROM numbers(5) FETCH NEXT (1.5) ROWS ONLY",
        "SELECT number FROM numbers(5) FETCH NEXT (ALL) ROWS ONLY",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
