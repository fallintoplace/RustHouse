use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn unique_uses_distinct_projection() {
    let query = parse_select("SELECT UNIQUE number FROM numbers(4)").unwrap();
    assert!(query.distinct);
    assert_eq!(
        run("SELECT UNIQUE number FROM numbers(4)"),
        run("SELECT DISTINCT number FROM numbers(4)")
    );
}

#[test]
fn unique_keeps_distinct_projection_restrictions() {
    assert!(parse_select("SELECT UNIQUE COUNT(*) FROM numbers(4)").is_err());
}
