use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn generated_grouped_having_respects_count_and_order() {
    let sql = "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING count(*) >= 1 ORDER BY number LIMIT 4";
    assert_eq!(run(sql), "number\tcount\n0\t1\n1\t1\n2\t1\n3\t1\n");
}
