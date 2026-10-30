use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn sparse_billion_row_membership_has_exact_sum_and_count() {
    let sql = "SELECT sum(number), count(*) FROM numbers(1000000000) WHERE number IN (42, 99, 999999998)";
    let expected = "sum(number)\tcount(*)\n1000000139\t3\n";
    assert_eq!(run(sql), expected);
}
