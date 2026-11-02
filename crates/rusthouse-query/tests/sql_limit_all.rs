use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn limit_all_matches_unlimited_query() {
    assert_eq!(
        run("SELECT number FROM numbers(4) ORDER BY number LIMIT ALL"),
        run("SELECT number FROM numbers(4) ORDER BY number"),
    );
    assert_eq!(
        run("SELECT number FROM numbers(4) ORDER BY number LIMIT ALL OFFSET 2"),
        run("SELECT number FROM numbers(4) ORDER BY number OFFSET 2"),
    );
    assert_eq!(
        run("SELECT number FROM numbers(0) LIMIT ALL"),
        run("SELECT number FROM numbers(0)"),
    );
}

#[test]
fn rejects_bad_limit_all_combinations() {
    for sql in [
        "SELECT number FROM numbers(4) LIMIT ALL, 2",
        "SELECT number FROM numbers(4) LIMIT ALL 2",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
