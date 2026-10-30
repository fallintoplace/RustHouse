use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn count_distinct_spans_dense_32bit_prefix_boundary() {
    let sql = "SELECT count(DISTINCT number) FROM numbers(70000) WHERE number >= 65530";
    assert_eq!(run(sql), "countDistinct(number)\n4470\n");
}
