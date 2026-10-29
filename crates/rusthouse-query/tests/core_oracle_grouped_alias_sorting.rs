use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn grouped_alias_sorting_paginates_by_aggregate_value() {
    let sql = "SELECT number, sum(number) AS total FROM numbers(40) GROUP BY number ORDER BY total DESC LIMIT 5 OFFSET 10";
    let mut expected = String::from("number\ttotal\n");
    for number in (25..30).rev() {
        expected.push_str(&format!("{number}\t{number}\n"));
    }
    assert_eq!(run(sql), expected);
}
