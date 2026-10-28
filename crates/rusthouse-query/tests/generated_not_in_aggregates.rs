use rusthouse_query::{execute, parse_select};
fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}
#[test]
fn billion_row_not_in_aggregates_use_closed_form_complements() {
    for (sql, expected) in [
        ("SELECT sum(number) FROM numbers(1000000000) WHERE number NOT IN (0, 1, 2)", "sum(number)\n499999999499999997\n"),
        ("SELECT min(number) FROM numbers(1000000000) WHERE number NOT IN (0, 1, 2)", "min(number)\n3\n"),
        ("SELECT max(number) FROM numbers(1000000000) WHERE number NOT IN (999999999)", "max(number)\n999999998\n"),
        ("SELECT min(number) FROM numbers(3) WHERE number NOT IN (0, 1, 2)", "min(number)\nNULL\n"),
        ("SELECT sum(number) FROM generate_series(10, 22, 3) WHERE number NOT IN (10, 16)", "sum(number)\n54\n"),
        ("SELECT sum(number) FROM numbers(30) WHERE number NOT IN (1, 2) AND number > 20", "sum(number)\n225\n"),
    ] {
        assert_eq!(run(sql), expected, "{sql}");
    }
}
