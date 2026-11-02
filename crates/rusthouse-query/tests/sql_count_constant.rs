use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn count_non_null_constants_matches_count_star() {
    for literal in ["1", "0", "-2", "'hello'", "''"] {
        let sql = format!("SELECT COUNT({literal}) FROM numbers(7)");
        assert_eq!(run(&sql), run("SELECT COUNT(*) FROM numbers(7)"), "{sql}");
    }
    assert_eq!(run("SELECT COUNT(1) FROM numbers(0)"), run("SELECT COUNT(*) FROM numbers(0)"));
}

#[test]
fn rejects_multiple_or_incomplete_count_constant_arguments() {
    for sql in [
        "SELECT COUNT(1, 2) FROM numbers(3)",
        "SELECT COUNT(1 + 2) FROM numbers(3)",
        "SELECT COUNT('a', 'b') FROM numbers(3)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
