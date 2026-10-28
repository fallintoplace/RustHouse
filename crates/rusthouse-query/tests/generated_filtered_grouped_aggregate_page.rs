use rusthouse_query::{execute, parse_select};
fn query(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn billion_row_filtered_grouped_aggregates_only_visit_requested_keys() {
    for (sql, expected) in [
        (
            "SELECT number, sum(number) FROM numbers(1000000000) WHERE number >= 999999990 GROUP BY number LIMIT 2 OFFSET 3",
            "number\tsum(number)\n999999993\t999999993\n999999994\t999999994\n",
        ),
        (
            "SELECT number, max(number) FROM numbers(1000000000) WHERE number BETWEEN 5 AND 6 GROUP BY number LIMIT 5",
            "number\tmax(number)\n5\t5\n6\t6\n",
        ),
        (
            "SELECT number, sum(number) FROM numbers(1000000000) WHERE number > 999999999 GROUP BY number LIMIT 5",
            "number\tsum(number)\n",
        ),
        (
            "SELECT number, min(number) FROM generate_series(10, 22, 3) WHERE number >= 13 GROUP BY number LIMIT 2 OFFSET 1",
            "number\tmin(number)\n16\t16\n19\t19\n",
        ),
        (
            "SELECT number, sum(number) FROM numbers(10) WHERE number != 9 GROUP BY number LIMIT 2",
            "number\tsum(number)\n0\t0\n1\t1\n",
        ),
    ] {
        assert_eq!(query(sql), expected, "{sql}");
    }
}
