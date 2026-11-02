use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn fetch_first_without_count_returns_one_row() {
    assert_eq!(
        run("SELECT number FROM numbers(5) ORDER BY number DESC FETCH FIRST ROW ONLY"),
        "number\n4\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(5) OFFSET 2 ROWS FETCH NEXT ROWS ONLY"),
        "number\n2\n"
    );
}

#[test]
fn explicit_fetch_count_still_works_and_only_is_required() {
    assert_eq!(
        run("SELECT number FROM numbers(5) FETCH FIRST 2 ROWS ONLY"),
        "number\n0\n1\n"
    );
    assert!(parse_select("SELECT number FROM numbers(5) FETCH FIRST ROW").is_err());
}
