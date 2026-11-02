use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn limit_all_means_unbounded_with_optional_offset() {
    let whole = parse_select("SELECT number FROM numbers(4) LIMIT ALL").unwrap();
    assert_eq!(whole.limit, None);
    assert_eq!(whole.offset, None);
    assert_eq!(run("SELECT number FROM numbers(4) LIMIT ALL"), "number\n0\n1\n2\n3\n");
    let page = parse_select("SELECT number FROM numbers(4) LIMIT ALL OFFSET 2").unwrap();
    assert_eq!(page.limit, None);
    assert_eq!(page.offset, Some(2));
    assert_eq!(run("SELECT number FROM numbers(4) LIMIT ALL OFFSET 2"), "number\n2\n3\n");
}

#[test]
fn limit_all_rejects_mixed_comma_and_numeric_syntax() {
    for sql in [
        "SELECT number FROM numbers(3) LIMIT ALL, 1",
        "SELECT number FROM numbers(3) LIMIT ALL 2",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
