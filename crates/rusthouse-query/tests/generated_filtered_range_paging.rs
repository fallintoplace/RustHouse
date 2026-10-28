use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn billion_row_range_pagination_without_materializing_candidates() {
    for (sql, expected) in [
        ("SELECT number FROM numbers(1000000000) WHERE number >= 999999990 ORDER BY number DESC LIMIT 3 OFFSET 2", "number\n999999997\n999999996\n999999995\n"),
        ("SELECT number FROM numbers(1000000000) WHERE number = 999999999 LIMIT 5", "number\n999999999\n"),
        ("SELECT number FROM numbers(1000000000) WHERE number BETWEEN 3 AND 9 LIMIT 2 OFFSET 3", "number\n6\n7\n"),
        ("SELECT number FROM numbers(1000000000) WHERE number > 999999999 LIMIT 3", "number\n"),
        ("SELECT number FROM generate_series(10, 22, 3) WHERE number >= 13 ORDER BY number DESC LIMIT 2", "number\n22\n19\n"),
        ("SELECT number FROM numbers(30) WHERE number != 29 ORDER BY number DESC LIMIT 2", "number\n28\n27\n"),
        ("SELECT number FROM numbers(20) WHERE number = 1 OR number = 15 LIMIT 2", "number\n1\n15\n"),
    ] {
        assert_eq!(query(sql), expected, "{sql}");
    }
}
