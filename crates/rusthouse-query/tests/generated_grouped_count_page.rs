use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn grouped_billion_row_page_keeps_unique_key_order() {
    for (sql, expected) in [
        (
            "SELECT number, count(*) FROM numbers(1000000000) GROUP BY number LIMIT 3 OFFSET 999999995",
            "number\tcount\n999999995\t1\n999999996\t1\n999999997\t1\n",
        ),
        (
            "SELECT number, count(*) FROM numbers(1000000000) GROUP BY number LIMIT 2 OFFSET 1000000000",
            "number\tcount\n",
        ),
        (
            "SELECT number, count(*) FROM numbers(10, 35, 5) GROUP BY number LIMIT 3 OFFSET 2",
            "number\tcount\n20\t1\n25\t1\n30\t1\n",
        ),
        (
            "SELECT number, count(*) FROM numbers(3) GROUP BY number LIMIT 0",
            "number\tcount\n",
        ),
    ] {
        assert_eq!(run(sql), expected, "{sql}");
    }
}
