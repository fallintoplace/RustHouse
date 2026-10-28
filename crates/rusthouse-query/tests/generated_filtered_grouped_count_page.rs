use rusthouse_query::{execute, parse_select};
fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}
#[test]
fn billion_row_filtered_unique_group_pages_skip_all_preceding_input() {
    for (sql, expected) in [
        (
            "SELECT number, count(*) FROM numbers(1000000000) WHERE number >= 999999990 GROUP BY number LIMIT 3 OFFSET 2",
            "number\tcount\n999999992\t1\n999999993\t1\n999999994\t1\n",
        ),
        (
            "SELECT number, count(*) FROM numbers(1000000000) WHERE number BETWEEN 3 AND 10 GROUP BY number LIMIT 2 OFFSET 3",
            "number\tcount\n6\t1\n7\t1\n",
        ),
        (
            "SELECT number, count(*) FROM numbers(1000000000) WHERE number < 1 GROUP BY number LIMIT 1 OFFSET 1",
            "number\tcount\n",
        ),
        (
            "SELECT number, count(*) FROM generate_series(10, 22, 3) WHERE number >= 13 GROUP BY number LIMIT 2 OFFSET 1",
            "number\tcount\n16\t1\n19\t1\n",
        ),
        (
            "SELECT number, count(*) FROM numbers(10) WHERE number != 9 GROUP BY number LIMIT 2",
            "number\tcount\n0\t1\n1\t1\n",
        ),
    ] {
        assert_eq!(run(sql), expected, "{sql}");
    }
}
