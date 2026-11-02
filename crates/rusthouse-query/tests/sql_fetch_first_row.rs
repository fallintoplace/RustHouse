use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn omitted_fetch_count_defaults_to_one_row() {
    let with_count = "SELECT number FROM numbers(6) ORDER BY number DESC FETCH FIRST 1 ROW ONLY";
    let without_count = "SELECT number FROM numbers(6) ORDER BY number DESC FETCH FIRST ROW ONLY";
    assert_eq!(run(without_count), run(with_count));
    assert_eq!(
        run("SELECT number FROM numbers(6) ORDER BY number OFFSET 2 ROWS FETCH NEXT ROW ONLY"),
        run("SELECT number FROM numbers(6) ORDER BY number LIMIT 1 OFFSET 2"),
    );
    assert_eq!(
        run("SELECT number FROM numbers(6) ORDER BY number FETCH FIRST ROWS ONLY"),
        run("SELECT number FROM numbers(6) ORDER BY number LIMIT 1"),
    );
}

#[test]
fn rejects_fetch_missing_row_and_only_keywords() {
    assert!(parse_select("SELECT number FROM numbers(6) FETCH FIRST ONLY").is_err());
    assert!(parse_select("SELECT number FROM numbers(6) FETCH FIRST ROW").is_err());
}
