use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn limit_accepts_parenthesized_integer() {
    let query = parse_select("SELECT number FROM numbers(5) LIMIT (2)").unwrap();
    assert_eq!(query.limit, Some(2));
    assert_eq!(run("SELECT number FROM numbers(5) LIMIT (2)"), "number\n0\n1\n");
    let page = parse_select("SELECT number FROM numbers(5) LIMIT (2), 1").unwrap();
    assert_eq!((page.offset, page.limit), (Some(2), Some(1)));
}

#[test]
fn limit_parentheses_reject_bad_counts_and_missing_close() {
    for sql in [
        "SELECT number FROM numbers(5) LIMIT (2",
        "SELECT number FROM numbers(5) LIMIT (-1)",
        "SELECT number FROM numbers(5) LIMIT (2.5)",
        "SELECT number FROM numbers(5) LIMIT (ALL)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
