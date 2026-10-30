use rusthouse_query::{parse_select, execute};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn hashed_large_in_lists_are_exact_for_generated_sources() {
    let list = (0..256).map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
    let included = format!("SELECT count(*) FROM numbers(512) WHERE number IN ({list})");
    assert_eq!(run(&included), "count\n256\n");
    let excluded = format!("SELECT count(*) FROM numbers(512) WHERE number NOT IN ({list})");
    assert_eq!(run(&excluded), "count\n256\n");
}

#[test]
fn in_lists_have_a_bounded_size_instead_of_quadratic_parsing() {
    let list = (0..4097).map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
    let sql = format!("SELECT number FROM numbers(1) WHERE number IN ({list})");
    assert!(parse_select(&sql).is_err());
}
