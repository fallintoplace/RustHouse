use rusthouse_query::{execute, parse_select};
fn run(sql: &str) -> String {
    let mut result = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut result).unwrap();
    String::from_utf8(result).unwrap()
}
#[test]
fn or_equality_aggregates_skip_billion_unmatched_generated_values() {
    for (sql, expected) in [
        ("SELECT sum(number) FROM numbers(1000000000) WHERE number = 4 OR number = 999999999", "sum(number)\n1000000003\n"),
        ("SELECT sum(number) FROM numbers(1000000000) WHERE number = 4 OR number = 4", "sum(number)\n4\n"),
        ("SELECT min(number) FROM numbers(1000000000) WHERE number IN (15, 20) OR number = 7", "min(number)\n7\n"),
        ("SELECT max(number) FROM numbers(1000000000) WHERE number = 10 OR number IN (20, 50)", "max(number)\n50\n"),
        ("SELECT sum(number) FROM numbers(30) WHERE number > 28 OR number = 2", "sum(number)\n31\n"),
    ] {
        assert_eq!(run(sql), expected, "{sql}");
    }
}
