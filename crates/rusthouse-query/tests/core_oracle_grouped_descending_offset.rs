use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn grouped_descending_page_keeps_exact_sorted_keys() {
    let sql = "SELECT number, count(*) FROM numbers(64) GROUP BY number ORDER BY number DESC LIMIT 5 OFFSET 3";
    assert_eq!(run(sql), "number\tcount\n60\t1\n59\t1\n58\t1\n57\t1\n56\t1\n");
}
