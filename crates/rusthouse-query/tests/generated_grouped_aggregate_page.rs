use rusthouse_query::{execute, parse_select};
fn run(sql: &str) -> String {
    let mut result = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut result).unwrap();
    String::from_utf8(result).unwrap()
}

#[test]
fn grouped_generated_aggregates_page_unique_keys_without_a_large_hash_table() {
    for (sql, expected) in [
        ("SELECT number, sum(number) FROM numbers(1000000000) GROUP BY number LIMIT 3 OFFSET 999999996", "number\tsum(number)\n999999996\t999999996\n999999997\t999999997\n999999998\t999999998\n"),
        ("SELECT number, max(number) FROM numbers(1000000000) GROUP BY number LIMIT 1 OFFSET 999999999", "number\tmax(number)\n999999999\t999999999\n"),
        ("SELECT number, min(number) FROM numbers(5) GROUP BY number LIMIT 2 OFFSET 5", "number\tmin(number)\n"),
        ("SELECT number, sum(number) FROM numbers(12, 10) GROUP BY number LIMIT 2 OFFSET 3", "number\tsum(number)\n15\t15\n16\t16\n"),
    ] {
        assert_eq!(run(sql), expected, "{sql}");
    }
}
