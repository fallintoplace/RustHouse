use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn grouped_multimetric_rows_have_consistent_sum_max_and_count() {
    let sql = "SELECT number, sum(number), max(number), count(number) FROM numbers(40) GROUP BY number ORDER BY number DESC LIMIT 4";
    let mut expected = String::from("number\tsum(number)\tmax(number)\tcount(number)\n");
    for number in (36..40).rev() {
        expected.push_str(&format!("{number}\t{number}\t{number}\t1\n"));
    }
    assert_eq!(run(sql), expected);
}
