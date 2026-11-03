use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn offset_accepts_parenthesized_integer() {
    let query = parse_select("SELECT number FROM numbers(5) LIMIT 2 OFFSET (2) ROWS").unwrap();
    assert_eq!((query.limit, query.offset), (Some(2), Some(2)));
    assert_eq!(
        run("SELECT number FROM numbers(5) LIMIT 2 OFFSET (2) ROWS"),
        "number\n2\n3\n"
    );
}

#[test]
fn offset_parentheses_reject_bad_counts_and_missing_close() {
    for sql in [
        "SELECT number FROM numbers(5) OFFSET (2",
        "SELECT number FROM numbers(5) OFFSET (-1)",
        "SELECT number FROM numbers(5) OFFSET (1.5)",
        "SELECT number FROM numbers(5) OFFSET (ALL)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
